use super::*;
use crate::entity::artist_known_album;
use reqwest::StatusCode;
use sea_orm::{ColumnTrait, QueryFilter};

struct Fixture {
    root: tempfile::TempDir,
    db: DatabaseConnection,
    state: AppState,
    downloader: Arc<GateDownloader>,
    worker: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new(source: Arc<FakeTidalCatalog>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let db = test_database().await;
        let directory = MusicDirectory::new(root.path().to_owned());
        let downloader = GateDownloader::new();
        let (queue, worker) =
            DownloadQueue::start(db.clone(), directory.clone(), downloader.clone());
        let state = AppState {
            tidal: source.clone(),
            queue,
            scan: ScanCoordinator::new(directory.clone(), db.clone(), source),
            db: db.clone(),
        };
        Self {
            root,
            db,
            state,
            downloader,
            worker,
        }
    }

    fn app(&self) -> Router {
        router(self.state.clone())
    }

    async fn request(&self, method: &str, path: &str, body: &str) -> Response {
        send_request(self.app(), method, path, body).await
    }

    async fn remove(&self, id: &str, status: StatusCode) {
        let response = self.request("DELETE", &format!("/artists/{id}"), "").await;
        assert_eq!(response.status(), status);
        if status == StatusCode::NO_CONTENT {
            assert!(
                to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }

    async fn add(&self, id: &str) {
        assert_eq!(
            self.request("POST", &format!("/albums/{id}"), "")
                .await
                .status(),
            StatusCode::OK
        );
    }

    async fn seed_artist(&self, id: &str, monitored: bool) -> artist::Model {
        let baseline = "2026-04-01T00:00:00Z".parse().unwrap();
        let attempt = "2026-04-02T00:00:00Z".parse().unwrap();
        let artist = artist::ActiveModel {
            id: Set(id.to_owned()),
            name: Set(format!("Saved {id}")),
            monitored: Set(monitored),
            monitoring_baseline_initialized_at: Set(Some(baseline)),
            last_check_attempt_at: Set(Some(attempt)),
            ..Default::default()
        }
        .insert(&self.db)
        .await
        .unwrap();
        artist_known_album::ActiveModel {
            artist_id: Set(id.to_owned()),
            normalized_title: Set(format!("known {id}")),
            release_type: Set("ALBUM".to_owned()),
        }
        .insert(&self.db)
        .await
        .unwrap();
        artist
    }

    async fn artist(&self, id: &str) -> Option<artist::Model> {
        artist::Entity::find_by_id(id).one(&self.db).await.unwrap()
    }

    async fn history(&self, id: &str) -> Vec<artist_known_album::Model> {
        artist_known_album::Entity::find()
            .filter(artist_known_album::Column::ArtistId.eq(id))
            .all(&self.db)
            .await
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

#[tokio::test]
async fn last_album_retains_monitored_artists_and_cleans_only_credited_unmonitored_orphans() {
    let mut removed = ScanAlbum::new("removed", "Removed", "2024");
    removed.artists.push(TidalArtist {
        id: "shared".to_owned(),
        name: "Shared artist".to_owned(),
        profile_image_url: None,
    });
    let mut remaining = ScanAlbum::new("remaining", "Remaining", "2024");
    remaining.artists = vec![removed.artists[2].clone()];
    let fixture = Fixture::new(Arc::new(FakeTidalCatalog {
        albums: vec![removed, remaining],
        ..Default::default()
    }))
    .await;
    for (id, monitored) in [
        ("z-primary", true),
        ("a-guest", false),
        ("shared", false),
        ("unrelated", false),
    ] {
        fixture.seed_artist(id, monitored).await;
    }
    fixture.add("removed").await;
    fixture.add("remaining").await;
    let before = catalog_rows(&fixture.db).await;
    let history = artist_known_album::Entity::find()
        .all(&fixture.db)
        .await
        .unwrap();

    let response = fixture.request("DELETE", "/albums/removed", "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({ "removed_artist_ids": ["a-guest"] })
    );
    let after = catalog_rows(&fixture.db).await;
    assert_eq!(
        after.0,
        before
            .0
            .into_iter()
            .filter(|row| row.id == "remaining")
            .collect::<Vec<_>>()
    );
    assert_eq!(
        after.1,
        before
            .1
            .into_iter()
            .filter(|row| row.id != "a-guest")
            .collect::<Vec<_>>()
    );
    assert_eq!(
        after.2,
        before
            .2
            .into_iter()
            .filter(|row| row.album_id == "remaining")
            .collect::<Vec<_>>()
    );
    let remaining_history = artist_known_album::Entity::find()
        .all(&fixture.db)
        .await
        .unwrap();
    assert_eq!(
        remaining_history,
        history
            .into_iter()
            .filter(|row| row.artist_id != "a-guest")
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn disabling_and_reenabling_empty_artist_preserves_identity_history_and_schedule() {
    let fixture = Fixture::new(Arc::new(FakeTidalCatalog::default())).await;
    let saved = fixture.seed_artist("empty", true).await;
    let history = fixture.history("empty").await;
    for monitored in [false, true] {
        assert_eq!(
            fixture
                .request(
                    "PATCH",
                    "/artists/empty",
                    &format!(r#"{{"monitored":{monitored}}}"#)
                )
                .await
                .status(),
            StatusCode::OK
        );
        let mut expected = saved.clone();
        expected.monitored = monitored;
        assert_eq!(fixture.artist("empty").await, Some(expected));
        assert_eq!(fixture.history("empty").await, history);
    }
}

#[tokio::test]
async fn explicit_removal_is_bodyless_scoped_and_does_not_touch_catalog_or_audio() {
    let fixture = Fixture::new(Arc::new(FakeTidalCatalog::default())).await;
    insert_test_album(&fixture.db, "owned").await;
    let unrelated = fixture.seed_artist("unrelated", false).await;
    let unrelated_history = fixture.history("unrelated").await;
    let audio = fixture.root.path().join("track.flac");
    tokio::fs::write(&audio, b"existing audio").await.unwrap();
    for monitored in [false, true] {
        fixture.seed_artist("empty", monitored).await;
        let before = catalog_rows(&fixture.db).await;
        fixture.remove("empty", StatusCode::NO_CONTENT).await;
        assert!(fixture.artist("empty").await.is_none());
        assert!(fixture.history("empty").await.is_empty());
        assert_eq!(fixture.artist("unrelated").await, Some(unrelated.clone()));
        assert_eq!(fixture.history("unrelated").await, unrelated_history);
        let after = catalog_rows(&fixture.db).await;
        assert_eq!(after.0, before.0);
        assert_eq!(after.2, before.2);
        assert_eq!(tokio::fs::read(&audio).await.unwrap(), b"existing audio");
        fixture.remove("empty", StatusCode::NOT_FOUND).await;
    }
}

#[tokio::test]
async fn stale_page_and_concurrent_removals_cannot_delete_any_credited_artist() {
    let fixture = Fixture::new(Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("added", "Added", "2024")],
        ..Default::default()
    }))
    .await;
    fixture.seed_artist("a-guest", false).await;
    assert_eq!(
        fixture
            .request("GET", "/artists/a-guest", "")
            .await
            .status(),
        StatusCode::OK
    );
    fixture.add("added").await;
    let before = catalog_rows(&fixture.db).await;
    let history = fixture.history("a-guest").await;
    let (first, second) = tokio::join!(
        fixture.request("DELETE", "/artists/a-guest", ""),
        fixture.request("DELETE", "/artists/z-primary", ""),
    );
    assert_eq!(first.status(), StatusCode::CONFLICT);
    assert_eq!(second.status(), StatusCode::CONFLICT);
    assert_eq!(catalog_rows(&fixture.db).await, before);
    assert_eq!(fixture.history("a-guest").await, history);
}

#[tokio::test]
async fn pending_add_can_reintroduce_removed_artist_with_fresh_monitoring_state() {
    let gate = Arc::new(Semaphore::new(0));
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("added", "Added", "2024")],
        metadata_gate: Some(gate.clone()),
        ..Default::default()
    });
    let fixture = Fixture::new(source.clone()).await;
    fixture.seed_artist("z-primary", true).await;
    let app = fixture.app();
    let addition =
        tokio::spawn(async move { send_request(app, "POST", "/albums/added", "").await });
    tokio::time::timeout(Duration::from_secs(5), source.metadata_started.notified())
        .await
        .unwrap();
    fixture.remove("z-primary", StatusCode::NO_CONTENT).await;
    assert!(fixture.history("z-primary").await.is_empty());
    gate.add_permits(1);
    assert_eq!(addition.await.unwrap().status(), StatusCode::OK);
    let (_, artists, credits) = catalog_rows(&fixture.db).await;
    assert_eq!(
        credits
            .iter()
            .map(|row| (&*row.artist_id, row.position))
            .collect::<Vec<_>>(),
        vec![("z-primary", 0), ("a-guest", 1)]
    );
    assert_eq!(artists.len(), 2);
    for artist in artists {
        assert!(!artist.monitored);
        assert!(artist.monitoring_baseline_initialized_at.is_none());
        assert!(artist.last_check_attempt_at.is_none());
        assert!(fixture.history(&artist.id).await.is_empty());
    }
    assert_eq!(
        fixture.artist("z-primary").await.unwrap().name,
        "Primary Artist"
    );
    fixture.remove("z-primary", StatusCode::CONFLICT).await;
}

#[tokio::test]
async fn database_failure_returns_500_without_partial_history_removal() {
    let fixture = Fixture::new(Arc::new(FakeTidalCatalog::default())).await;
    let saved = fixture.seed_artist("empty", false).await;
    let history = fixture.history("empty").await;
    fixture.db.execute_unprepared(
        "CREATE TRIGGER fail_artist_delete BEFORE DELETE ON artist BEGIN SELECT RAISE(ABORT, 'test deletion failure'); END"
    ).await.unwrap();
    fixture
        .remove("empty", StatusCode::INTERNAL_SERVER_ERROR)
        .await;
    assert_eq!(fixture.artist("empty").await, Some(saved));
    assert_eq!(fixture.history("empty").await, history);
}

#[tokio::test]
async fn running_and_queued_downloads_and_active_scan_do_not_block_or_get_cancelled() {
    let fixture = Fixture::new(Arc::new(FakeTidalCatalog::default())).await;
    fixture.seed_artist("empty", true).await;
    for id in ["running", "queued"] {
        insert_test_album(&fixture.db, id).await;
        fixture.state.queue.enqueue(id.to_owned()).await.unwrap();
        if id == "running" {
            tokio::time::timeout(
                Duration::from_secs(5),
                fixture.downloader.started.notified(),
            )
            .await
            .unwrap();
        }
    }
    let before = downloads(&fixture.app()).await;
    let catalog = catalog_rows(&fixture.db).await;
    assert_eq!(
        fixture.request("POST", "/catalog/scan", "").await.status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(scan_status(&fixture.app()).await["phase"], "scanning");
    fixture.remove("empty", StatusCode::NO_CONTENT).await;
    assert_eq!(downloads(&fixture.app()).await, before);
    assert_eq!(scan_status(&fixture.app()).await["phase"], "scanning");
    let after = catalog_rows(&fixture.db).await;
    assert_eq!(after.0, catalog.0);
    assert_eq!(after.2, catalog.2);
    fixture.downloader.release.notify_one();
    tokio::time::timeout(
        Duration::from_secs(5),
        fixture.downloader.started.notified(),
    )
    .await
    .unwrap();
    fixture.downloader.release.notify_one();
    let jobs = wait_for_history(&fixture.app(), 2).await;
    assert!(
        jobs["history"]
            .as_array()
            .unwrap()
            .iter()
            .all(|job| job["status"] == "succeeded")
    );
}
