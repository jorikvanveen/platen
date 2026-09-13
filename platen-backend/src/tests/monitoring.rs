use super::*;
use crate::{entity::artist_known_album, services::monitoring as monitor};
use chrono::{DateTime, TimeDelta, Utc};
use reqwest::StatusCode;
use sea_orm::{ColumnTrait, QueryFilter};
use serde_json::Value;

async fn json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

async fn add(app: &Router, id: &str) {
    let response = send_request(app.clone(), "POST", &format!("/albums/{id}"), "").await;
    assert_eq!(response.status(), StatusCode::OK);
}

async fn delete(app: &Router, id: &str) {
    let response = send_request(app.clone(), "DELETE", &format!("/albums/{id}"), "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
}

async fn preference(app: &Router, id: &str, monitored: bool) -> Value {
    let response = send_request(
        app.clone(),
        "PATCH",
        &format!("/artists/{id}"),
        &format!(r#"{{"monitored":{monitored}}}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let dto = json(response).await;
    assert_eq!(dto["id"], id);
    assert_eq!(dto["monitored"], monitored);
    for internal in [
        "monitoring_baseline_initialized_at",
        "last_check_attempt_at",
        "known_albums",
    ] {
        assert!(
            dto.get(internal).is_none(),
            "internal field leaked: {internal}"
        );
    }
    dto
}

async fn stored_artist(db: &DatabaseConnection, id: &str) -> artist::Model {
    artist::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

async fn wait_for_monitoring_baselines(db: &DatabaseConnection, artist_ids: &[&str]) {
    tokio::time::timeout(Duration::from_secs(5), async {
        for id in artist_ids {
            while stored_artist(db, id)
                .await
                .monitoring_baseline_initialized_at
                .is_none()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    })
    .await
    .unwrap();
}

async fn known(db: &DatabaseConnection, id: &str) -> Vec<(String, String)> {
    artist_known_album::Entity::find()
        .filter(artist_known_album::Column::ArtistId.eq(id))
        .order_by_asc(artist_known_album::Column::NormalizedTitle)
        .order_by_asc(artist_known_album::Column::ReleaseType)
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.normalized_title, row.release_type))
        .collect()
}

async fn no_jobs(app: &Router) {
    let jobs = downloads(app).await;
    assert_eq!(jobs["active"], serde_json::json!([]));
    assert_eq!(jobs["history"], serde_json::json!([]));
}

fn time() -> DateTime<Utc> {
    "2026-04-01T00:00:00Z".parse().unwrap()
}

fn observation(source: &FakeTidalCatalog, id: &str, albums: Vec<TidalAlbum>) {
    source
        .discography_responses_by_artist_id
        .lock()
        .unwrap()
        .insert(id.to_owned(), Ok(albums));
}

fn monitoring_app(
    db: &DatabaseConnection,
    music_root: &Path,
    source: Arc<FakeTidalCatalog>,
) -> (Router, DownloadQueue, tokio::task::JoinHandle<()>) {
    let music_directory = MusicDirectory::new(music_root.to_owned());
    let (queue, worker) =
        DownloadQueue::start(db.clone(), music_directory.clone(), GateDownloader::new());
    let app = router(AppState {
        tidal: source.clone(),
        queue: queue.clone(),
        scan: ScanCoordinator::new(music_directory, db.clone(), source),
        db: db.clone(),
    });
    (app, queue, worker)
}

async fn sweep(
    db: &DatabaseConnection,
    source: &FakeTidalCatalog,
    queue: &DownloadQueue,
    now: DateTime<Utc>,
) {
    tokio::time::timeout(
        Duration::from_secs(5),
        monitor::check_due_artists(db, source, queue, || now),
    )
    .await
    .expect("monitoring check deadlocked")
    .unwrap();
}

async fn check(
    db: &DatabaseConnection,
    source: &FakeTidalCatalog,
    queue: &DownloadQueue,
    hours: i64,
) {
    sweep(db, source, queue, time() + TimeDelta::hours(hours)).await;
}

async fn seed_empty(
    db: &DatabaseConnection,
    source: &FakeTidalCatalog,
    queue: &DownloadQueue,
    artists: &[(&str, bool)],
) {
    for (id, monitored) in artists {
        artist::ActiveModel {
            id: Set((*id).to_owned()),
            name: Set((*id).to_owned()),
            monitored: Set(*monitored),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
        observation(source, id, vec![]);
    }
    check(db, source, queue, 0).await;
    assert!(queue.is_empty().await);
}

async fn active_album_ids(queue: &DownloadQueue) -> Vec<String> {
    let mut ids: Vec<_> = queue
        .snapshot()
        .await
        .0
        .into_iter()
        .map(|job| job.album_id)
        .collect();
    ids.sort();
    ids
}

async fn catalog_album_ids(db: &DatabaseConnection) -> Vec<String> {
    catalog_rows(db)
        .await
        .0
        .into_iter()
        .map(|album| album.id)
        .collect()
}

#[tokio::test]
async fn issue55_six_hour_checks_remember_disabled_observations_without_catch_up() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![
            ScanAlbum::new("101", "While disabled", "2026"),
            ScanAlbum::new("102", "New release", "2026"),
            ScanAlbum::new("103", "Rediscovered archive", "1970"),
        ],
        ..Default::default()
    });
    let (app, queue, worker) = monitoring_app(&db, music.path(), source.clone());
    seed_empty(
        &db,
        &source,
        &queue,
        &[("z-primary", false), ("a-guest", true)],
    )
    .await;
    for id in ["z-primary", "a-guest"] {
        observation(&source, id, vec![source.albums[0].album.clone()]);
    }
    check(&db, &source, &queue, 5).await;
    assert_eq!(source.discography_calls.lock().unwrap().len(), 2);
    preference(&app, "a-guest", false).await;
    for id in ["z-primary", "a-guest"] {
        assert_eq!(
            stored_artist(&db, id).await.last_check_attempt_at,
            Some(time().fixed_offset())
        );
    }
    check(&db, &source, &queue, 6).await;
    no_jobs(&app).await;
    for id in ["z-primary", "a-guest"] {
        assert_eq!(
            known(&db, id).await,
            vec![("while disabled".into(), "ALBUM".into())]
        );
        assert_eq!(
            stored_artist(&db, id).await.last_check_attempt_at,
            Some((time() + TimeDelta::hours(6)).fixed_offset())
        );
    }
    preference(&app, "z-primary", true).await;
    check(&db, &source, &queue, 11).await;
    assert_eq!(source.discography_calls.lock().unwrap().len(), 4);
    check(&db, &source, &queue, 12).await;
    assert_eq!(active_album_ids(&queue).await, ["102", "103"]);
    assert_eq!(catalog_album_ids(&db).await, ["102", "103"]);
    for id in ["z-primary", "a-guest"] {
        assert_eq!(known(&db, id).await.len(), 3);
        assert_eq!(
            stored_artist(&db, id)
                .await
                .monitoring_baseline_initialized_at,
            Some(time().fixed_offset())
        );
    }
    preference(&app, "z-primary", false).await;
    assert_eq!(active_album_ids(&queue).await, ["102", "103"]);
    worker.abort();
}

#[tokio::test]
async fn issue55_history_accumulates_and_known_pairs_do_not_upgrade_or_reappear_as_new() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let original = ScanAlbum::new("101", "  Known\u{a0}  Record ", "2020");
    let mut upgrade = ScanAlbum::new("102", "known record", "2020");
    upgrade.album.media_tags = Some(vec!["HIRES_LOSSLESS".into()]);
    let mut single = ScanAlbum::new("103", "KNOWN RECORD", "2020");
    single.album.r#type = "SINGLE".into();
    let clean = ScanAlbum::new("104", "New pair", "2026");
    let mut explicit = ScanAlbum::new("105", "New pair", "2026");
    explicit.album.explicit = Some(true);
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![original, upgrade, single, clean, explicit],
        ..Default::default()
    });
    let (app, queue, worker) = monitoring_app(&db, music.path(), source.clone());
    seed_empty(&db, &source, &queue, &[("z-primary", false)]).await;
    observation(&source, "z-primary", vec![source.albums[0].album.clone()]);
    check(&db, &source, &queue, 6).await;
    preference(&app, "z-primary", true).await;
    observation(&source, "z-primary", vec![]);
    check(&db, &source, &queue, 12).await;
    assert_eq!(
        known(&db, "z-primary").await,
        vec![("known record".into(), "ALBUM".into())]
    );
    check(&db, &source, &queue, 18).await;
    assert_eq!(catalog_album_ids(&db).await, ["103", "105"]);
    assert_eq!(active_album_ids(&queue).await, ["103", "105"]);
    assert_eq!(
        known(&db, "z-primary").await,
        vec![
            ("known record".into(), "ALBUM".into()),
            ("known record".into(), "SINGLE".into()),
            ("new pair".into(), "ALBUM".into()),
        ]
    );
    assert!(!stored_artist(&db, "a-guest").await.monitored);
    let credits = catalog_rows(&db).await.2;
    for album_id in ["103", "105"] {
        assert_eq!(
            credits
                .iter()
                .filter(|credit| credit.album_id == album_id)
                .map(|credit| (credit.artist_id.as_str(), credit.position))
                .collect::<Vec<_>>(),
            vec![("z-primary", 0), ("a-guest", 1)]
        );
    }
    let jobs = queue.snapshot().await.0;
    check(&db, &source, &queue, 24).await;
    assert_eq!(
        queue
            .snapshot()
            .await
            .0
            .iter()
            .map(|job| &job.id)
            .collect::<Vec<_>>(),
        jobs.iter().map(|job| &job.id).collect::<Vec<_>>()
    );
    worker.abort();
}

#[tokio::test]
async fn issue55_collaborators_reuse_catalog_editions_downloads_and_active_jobs_with_artist_scoping()
 {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let mut unrelated = ScanAlbum::new("109", "Shared", "2020");
    unrelated.artists = vec![TidalArtist {
        id: "unrelated".into(),
        name: "Unrelated".into(),
        profile_image_url: None,
    }];
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![
            ScanAlbum::new("101", "Shared", "2026"),
            ScanAlbum::new("102", "Existing", "2026"),
            ScanAlbum::new("103", "Existing", "2020"),
            ScanAlbum::new("104", "Downloaded", "2026"),
            ScanAlbum::new("105", "Active", "2026"),
            unrelated,
            ScanAlbum::new("110", "Shared", "2026"),
        ],
        ..Default::default()
    });
    let (app, queue, worker) = monitoring_app(&db, music.path(), source.clone());
    seed_empty(
        &db,
        &source,
        &queue,
        &[("z-primary", true), ("a-guest", true)],
    )
    .await;
    for id in ["103", "104", "105", "109"] {
        add(&app, id).await;
    }
    let mut downloaded: album::ActiveModel = album::Entity::find_by_id("104")
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    downloaded.relative_path = Set(Some("Primary Artist/Downloaded".into()));
    downloaded.update(&db).await.unwrap();
    let existing_job = queue.enqueue("105".into()).await.unwrap();
    let visible = [0, 1, 3, 4]
        .map(|index| source.albums[index].album.clone())
        .to_vec();
    observation(&source, "z-primary", visible.clone());
    let mut guest_visible = visible;
    guest_visible[0] = source.albums[6].album.clone();
    observation(&source, "a-guest", guest_visible);
    check(&db, &source, &queue, 6).await;
    let catalog_ids = catalog_album_ids(&db).await;
    let shared_ids: Vec<_> = catalog_ids
        .iter()
        .filter(|id| matches!(id.as_str(), "101" | "110"))
        .cloned()
        .collect();
    assert_eq!(shared_ids.len(), 1);
    assert_eq!(catalog_ids.len(), 5);
    for id in ["103", "104", "105", "109"] {
        assert!(catalog_ids.iter().any(|stored| stored == id));
    }
    let mut expected_jobs = vec![shared_ids[0].clone(), "103".into(), "105".into()];
    expected_jobs.sort();
    assert_eq!(active_album_ids(&queue).await, expected_jobs);
    assert_eq!(
        queue
            .snapshot()
            .await
            .0
            .iter()
            .find(|job| job.album_id == "105")
            .unwrap()
            .id,
        existing_job.id
    );
    for id in ["z-primary", "a-guest"] {
        assert_eq!(known(&db, id).await.len(), 4);
    }
    worker.abort();
}

#[tokio::test]
async fn issue55_catalog_failure_and_queue_rejection_are_observed_once_not_retried() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![
            ScanAlbum::new("101", "Broken catalog add", "2026"),
            ScanAlbum::new("102", "Continues", "2026"),
            ScanAlbum::new("103", "Rejected", "2026"),
        ],
        ..Default::default()
    });
    let (_app, queue, worker) = monitoring_app(&db, music.path(), source.clone());
    seed_empty(&db, &source, &queue, &[("z-primary", true)]).await;
    db.execute_unprepared("CREATE TRIGGER fail_monitor_album BEFORE INSERT ON album WHEN NEW.id = '101' BEGIN SELECT RAISE(ABORT, 'test catalog failure'); END").await.unwrap();
    observation(
        &source,
        "z-primary",
        source.albums[..2]
            .iter()
            .map(|record| record.album.clone())
            .collect(),
    );
    check(&db, &source, &queue, 6).await;
    assert_eq!(known(&db, "z-primary").await.len(), 2);
    assert_eq!(catalog_album_ids(&db).await, ["102"]);
    assert_eq!(active_album_ids(&queue).await, ["102"]);
    db.execute_unprepared("DROP TRIGGER fail_monitor_album")
        .await
        .unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    check(&db, &source, &queue, 12).await;
    assert_eq!(known(&db, "z-primary").await.len(), 3);
    assert_eq!(catalog_album_ids(&db).await, ["102", "103"]);
    assert_eq!(active_album_ids(&queue).await, ["102"]);
    let calls = source.metadata_calls.lock().unwrap().clone();
    let (_app, replacement_queue, replacement_worker) =
        monitoring_app(&db, music.path(), source.clone());
    check(&db, &source, &replacement_queue, 18).await;
    assert!(replacement_queue.is_empty().await);
    assert_eq!(*source.metadata_calls.lock().unwrap(), calls);
    assert_eq!(catalog_album_ids(&db).await, ["102", "103"]);
    replacement_worker.abort();
}

#[tokio::test]
async fn issue55_accepted_failed_cancelled_and_lost_jobs_are_never_resubmitted() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![
            ScanAlbum::new("101", "Failure", "2026"),
            ScanAlbum::new("102", "Running", "2026"),
            ScanAlbum::new("103", "Cancelled", "2026"),
        ],
        ..Default::default()
    });
    let (queue, worker) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(music.path().to_owned()),
        Arc::new(FailFirstDownloader {
            calls: AtomicUsize::new(0),
        }),
    );
    seed_empty(&db, &source, &queue, &[("z-primary", true)]).await;
    observation(&source, "z-primary", vec![source.albums[0].album.clone()]);
    check(&db, &source, &queue, 6).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while queue.snapshot().await.1.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        queue.snapshot().await.1[0].status,
        crate::services::download_queue::JobStatus::Failed
    );
    check(&db, &source, &queue, 7).await;
    assert_eq!(queue.snapshot().await.1.len(), 1);
    worker.abort();
    let downloader = GateDownloader::new();
    let (queue, worker) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(music.path().to_owned()),
        downloader.clone(),
    );
    check(&db, &source, &queue, 12).await;
    tokio::time::timeout(Duration::from_secs(5), downloader.started.notified())
        .await
        .unwrap();
    let jobs = queue.snapshot().await.0;
    assert_eq!(jobs.len(), 2);
    let queued = jobs
        .iter()
        .find(|job| job.status == crate::services::download_queue::JobStatus::Queued)
        .unwrap();
    queue.cancel(&queued.id).await.unwrap();
    check(&db, &source, &queue, 18).await;
    assert_eq!(queue.snapshot().await.0.len(), 1);
    assert_eq!(queue.snapshot().await.1.len(), 1);
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    let (_app, queue, worker) = monitoring_app(&db, music.path(), source.clone());
    check(&db, &source, &queue, 24).await;
    assert!(queue.is_empty().await);
    assert_eq!(known(&db, "z-primary").await.len(), 3);
    worker.abort();
}

#[tokio::test]
async fn issue55_gated_fetch_uses_processing_preference_and_metadata_rechecks_before_add() {
    for reenable in [false, true] {
        let db = test_database().await;
        let music = tempfile::tempdir().unwrap();
        let gate = Arc::new(Semaphore::new(1));
        let source = Arc::new(FakeTidalCatalog {
            albums: vec![ScanAlbum::new("101", "Discovered", "2026")],
            discography_gates_by_artist_id: HashMap::from([("z-primary".into(), gate.clone())]),
            ..Default::default()
        });
        let (app, queue, worker) = monitoring_app(&db, music.path(), source.clone());
        seed_empty(&db, &source, &queue, &[("z-primary", true)]).await;
        // Consume the baseline notification so the next one belongs to the blocked fetch.
        source.discography_started.notified().await;
        let attempt = tokio::spawn({
            let db = db.clone();
            let source = source.clone();
            let queue = queue.clone();
            async move { check(&db, &source, &queue, 6).await }
        });
        tokio::time::timeout(
            Duration::from_secs(5),
            source.discography_started.notified(),
        )
        .await
        .unwrap();
        assert_eq!(
            stored_artist(&db, "z-primary").await.last_check_attempt_at,
            Some((time() + TimeDelta::hours(6)).fixed_offset())
        );
        preference(&app, "z-primary", false).await;
        if reenable {
            preference(&app, "z-primary", true).await;
        }
        gate.add_permits(1);
        attempt.await.unwrap();
        assert_eq!(known(&db, "z-primary").await.len(), 1);
        assert_eq!(catalog_album_ids(&db).await.len(), usize::from(reenable));
        assert_eq!(active_album_ids(&queue).await.len(), usize::from(reenable));
        preference(&app, "z-primary", true).await;
        gate.add_permits(1);
        check(&db, &source, &queue, 12).await;
        assert_eq!(active_album_ids(&queue).await.len(), usize::from(reenable));
        worker.abort();
    }

    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("101", "Discovered", "2026")],
        metadata_gate: Some(gate.clone()),
        ..Default::default()
    });
    let (app, queue, worker) = monitoring_app(&db, music.path(), source.clone());
    seed_empty(&db, &source, &queue, &[("z-primary", true)]).await;
    let attempt = tokio::spawn({
        let db = db.clone();
        let source = source.clone();
        let queue = queue.clone();
        async move { check(&db, &source, &queue, 6).await }
    });
    tokio::time::timeout(Duration::from_secs(5), source.metadata_started.notified())
        .await
        .unwrap();
    preference(&app, "z-primary", false).await;
    gate.add_permits(1);
    attempt.await.unwrap();
    assert_eq!(known(&db, "z-primary").await.len(), 1);
    assert!(catalog_album_ids(&db).await.is_empty());
    no_jobs(&app).await;
    preference(&app, "z-primary", true).await;
    check(&db, &source, &queue, 12).await;
    no_jobs(&app).await;
    worker.abort();
}

#[tokio::test]
async fn issue55_failed_complete_observation_preserves_history_then_due_retry_hands_off() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![
            ScanAlbum::new("101", "Known", "2020"),
            ScanAlbum::new("102", "New", "2026"),
        ],
        ..Default::default()
    });
    let (app, queue, worker) = monitoring_app(&db, music.path(), source.clone());
    seed_empty(&db, &source, &queue, &[("z-primary", false)]).await;
    observation(&source, "z-primary", vec![source.albums[0].album.clone()]);
    check(&db, &source, &queue, 6).await;
    preference(&app, "z-primary", true).await;
    let previous = known(&db, "z-primary").await;
    // Tidal maps both a failed page request and the page limit to UnexpectedResponse.
    for (hour, error) in [
        (12, TidalError::UnexpectedResponse),
        (18, TidalError::ArtistAlbumsTimeout),
    ] {
        source
            .discography_responses_by_artist_id
            .lock()
            .unwrap()
            .insert("z-primary".into(), Err(error));
        check(&db, &source, &queue, hour).await;
        assert_eq!(known(&db, "z-primary").await, previous);
        no_jobs(&app).await;
        assert!(catalog_album_ids(&db).await.is_empty());
        let attempts = source.discography_calls.lock().unwrap().len();
        check(&db, &source, &queue, hour + 5).await;
        assert_eq!(source.discography_calls.lock().unwrap().len(), attempts);
    }
    check(&db, &source, &queue, 24).await;
    assert_eq!(known(&db, "z-primary").await.len(), 2);
    assert_eq!(catalog_album_ids(&db).await, ["102"]);
    assert_eq!(active_album_ids(&queue).await, ["102"]);
    assert_eq!(
        stored_artist(&db, "z-primary")
            .await
            .monitoring_baseline_initialized_at,
        Some(time().fixed_offset())
    );
    worker.abort();
}

#[tokio::test]
async fn issue55_startup_checks_overdue_once_and_reopen_preserves_schedule_history_and_handoffs() {
    let storage = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        storage.path().join("catalog.sqlite").display()
    );
    let db = Database::connect(&database_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let music = tempfile::tempdir().unwrap();
    let discovered = ScanAlbum::new("101", "Discovered during downtime", "2026");
    let later = ScanAlbum::new("102", "Next observation", "1970");
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![discovered.clone(), later],
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    seed_empty(
        &db,
        &source,
        &queue,
        &[("z-primary", true), ("a-guest", false)],
    )
    .await;
    let overdue_at = Utc::now() - TimeDelta::days(10);
    for id in ["z-primary", "a-guest"] {
        let mut overdue: artist::ActiveModel = stored_artist(&db, id).await.into();
        overdue.last_check_attempt_at = Set(Some(overdue_at.fixed_offset()));
        overdue.update(&db).await.unwrap();
        observation(&source, id, vec![discovered.album.clone()]);
    }
    source.discography_calls.lock().unwrap().clear();
    let monitor_worker = monitor::start(db.clone(), source.clone(), queue.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        while queue.is_empty().await || known(&db, "a-guest").await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    monitor_worker.abort();
    assert!(monitor_worker.await.unwrap_err().is_cancelled());
    let mut startup_calls = source.discography_calls.lock().unwrap().clone();
    startup_calls.sort();
    assert_eq!(startup_calls, ["a-guest", "z-primary"]);
    assert_eq!(active_album_ids(&queue).await, ["101"]);
    let saved_artist = stored_artist(&db, "z-primary").await;
    let saved_history = known(&db, "z-primary").await;
    assert_eq!(known(&db, "a-guest").await, saved_history);
    assert_eq!(
        saved_artist.monitoring_baseline_initialized_at,
        Some(time().fixed_offset())
    );
    assert!(saved_artist.last_check_attempt_at.unwrap() > overdue_at);
    queue_worker.abort();
    assert!(queue_worker.await.unwrap_err().is_cancelled());
    drop(app);
    drop(queue);
    db.close().await.unwrap();

    let db = Database::connect(&database_url).await.unwrap();
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    assert_eq!(stored_artist(&db, "z-primary").await, saved_artist);
    assert_eq!(known(&db, "z-primary").await, saved_history);
    assert_eq!(known(&db, "a-guest").await, saved_history);
    let last_attempt = saved_artist
        .last_check_attempt_at
        .unwrap()
        .with_timezone(&Utc);
    let earliest_attempt = last_attempt.min(
        stored_artist(&db, "a-guest")
            .await
            .last_check_attempt_at
            .unwrap()
            .with_timezone(&Utc),
    );
    monitor::check_due_artists(&db, source.as_ref(), &queue, || {
        earliest_attempt + TimeDelta::hours(6) - TimeDelta::seconds(1)
    })
    .await
    .unwrap();
    assert_eq!(source.discography_calls.lock().unwrap().len(), 2);
    no_jobs(&app).await;
    monitor::check_due_artists(&db, source.as_ref(), &queue, || {
        last_attempt + TimeDelta::hours(6)
    })
    .await
    .unwrap();
    assert_eq!(active_album_ids(&queue).await, ["102"]);
    assert_eq!(known(&db, "z-primary").await.len(), 2);
    assert_eq!(
        stored_artist(&db, "z-primary")
            .await
            .monitoring_baseline_initialized_at,
        saved_artist.monitoring_baseline_initialized_at
    );
    queue_worker.abort();
    let _ = queue_worker.await;
    drop(app);
    drop(queue);
    db.close().await.unwrap();
}

#[tokio::test]
async fn issue54_add_and_scan_initialize_disabled_collaborators_without_blocking_catalog_writes() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let mut added = ScanAlbum::new("added", "Added", "2024");
    added.artists.push(TidalArtist {
        id: "new-guest".to_owned(),
        name: "New guest".to_owned(),
        profile_image_url: None,
    });
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![
            ScanAlbum::new("owned", "Owned", "2024"),
            added,
            ScanAlbum::new("scanned", "Scanned", "2024"),
        ],
        discography_gates_by_artist_id: HashMap::from([("a-guest".to_owned(), gate.clone())]),
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    for id in ["a-guest", "z-primary"] {
        let artist = stored_artist(&db, id).await;
        assert!(!artist.monitored);
        assert!(artist.monitoring_baseline_initialized_at.is_none());
        assert!(artist.last_check_attempt_at.is_none());
    }
    let worker = monitor::start(db.clone(), source.clone(), queue);
    tokio::time::timeout(
        Duration::from_secs(5),
        source.discography_started.notified(),
    )
    .await
    .unwrap();
    // SQLite has one connection here: these writes also prove the fetch holds no transaction.
    preference(&app, "z-primary", true).await;
    add(&app, "added").await;
    assert!(!stored_artist(&db, "new-guest").await.monitored);
    create_scan_audio(music.path(), &["Primary Artist/Scanned"]).await;
    let scan = run_scan(&app).await;
    assert_eq!(scan["phase"], "completed");
    assert!(!stored_artist(&db, "a-guest").await.monitored);
    gate.add_permits(1);
    wait_for_monitoring_baselines(&db, &["a-guest", "z-primary"]).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::time::resume();
    wait_for_monitoring_baselines(&db, &["new-guest"]).await;
    assert_eq!(catalog_rows(&db).await.0.len(), 3);
    for id in ["a-guest", "z-primary"] {
        assert_eq!(known(&db, id).await.len(), 3);
    }
    assert_eq!(
        known(&db, "new-guest").await,
        vec![("added".into(), "ALBUM".into())]
    );
    no_jobs(&app).await;
    worker.abort();
    queue_worker.abort();
}

#[tokio::test]
async fn issue54_scan_introduces_disabled_artists_and_successful_empty_monitoring_baselines() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    create_scan_audio(music.path(), &["Primary Artist/Owned"]).await;
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("owned", "Owned", "2024")],
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    assert_eq!(run_scan(&app).await["phase"], "completed");
    for id in ["a-guest", "z-primary"] {
        assert!(!stored_artist(&db, id).await.monitored);
        observation(&source, id, vec![]);
    }
    sweep(&db, &source, &queue, time()).await;
    for id in ["a-guest", "z-primary"] {
        let artist = stored_artist(&db, id).await;
        assert_eq!(
            artist.monitoring_baseline_initialized_at,
            Some(time().fixed_offset())
        );
        assert_eq!(artist.last_check_attempt_at, Some(time().fixed_offset()));
        assert!(known(&db, id).await.is_empty());
    }
    assert_eq!(catalog_rows(&db).await.0.len(), 1);
    no_jobs(&app).await;
    queue_worker.abort();
}

#[tokio::test]
async fn issue54_monitoring_baseline_is_unfiltered_normalized_typed_and_scoped_per_artist() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("owned", "Owned", "2024")],
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    let mut single = ScanAlbum::new("single", "Old Record", "2000").album;
    single.r#type = "SINGLE".to_owned();
    observation(
        &source,
        "z-primary",
        vec![
            source.albums[0].album.clone(),
            ScanAlbum::new("old", "  OLD\u{a0}  Record  ", "2000").album,
            ScanAlbum::new("duplicate", "old record", "2000").album,
            single,
            ScanAlbum::new("suffix", "Old Record (Deluxe)", "2000").album,
            ScanAlbum::new("punctuation", "Old-Record", "2000").album,
        ],
    );
    observation(
        &source,
        "a-guest",
        vec![ScanAlbum::new("guest", "Old Record", "2000").album],
    );
    sweep(&db, &source, &queue, time()).await;
    assert_eq!(
        known(&db, "z-primary").await,
        vec![
            ("old record".into(), "ALBUM".into()),
            ("old record".into(), "SINGLE".into()),
            ("old record (deluxe)".into(), "ALBUM".into()),
            ("old-record".into(), "ALBUM".into()),
            ("owned".into(), "ALBUM".into()),
        ]
    );
    assert_eq!(
        known(&db, "a-guest").await,
        vec![("old record".into(), "ALBUM".into())]
    );
    assert_eq!(catalog_rows(&db).await.0.len(), 1);
    no_jobs(&app).await;
    queue_worker.abort();
}

#[tokio::test]
async fn issue54_failure_retries_only_when_due_and_first_success_survives_preference_changes() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("owned", "Owned", "2024")],
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    source
        .discography_responses_by_artist_id
        .lock()
        .unwrap()
        .insert("z-primary".into(), Err(TidalError::UnexpectedResponse));
    sweep(&db, &source, &queue, time()).await;
    let failed = stored_artist(&db, "z-primary").await;
    assert!(failed.monitoring_baseline_initialized_at.is_none());
    assert_eq!(failed.last_check_attempt_at, Some(time().fixed_offset()));
    assert!(known(&db, "z-primary").await.is_empty());
    observation(
        &source,
        "z-primary",
        vec![
            source.albums[0].album.clone(),
            ScanAlbum::new("intervening", "Intervening", "2026").album,
        ],
    );
    sweep(
        &db,
        &source,
        &queue,
        time() + TimeDelta::hours(6) - TimeDelta::seconds(1),
    )
    .await;
    assert_eq!(stored_artist(&db, "z-primary").await, failed);
    sweep(&db, &source, &queue, time() + TimeDelta::hours(6)).await;
    let initialized = stored_artist(&db, "z-primary").await;
    let history = known(&db, "z-primary").await;
    assert_eq!(history.len(), 2);
    assert_eq!(
        initialized.monitoring_baseline_initialized_at,
        Some((time() + TimeDelta::hours(6)).fixed_offset())
    );
    assert_eq!(
        initialized.last_check_attempt_at,
        Some((time() + TimeDelta::hours(6)).fixed_offset())
    );
    observation(
        &source,
        "z-primary",
        vec![ScanAlbum::new("later", "Later", "2026").album],
    );
    for monitored in [false, true, false, true] {
        preference(&app, "z-primary", monitored).await;
        let current = stored_artist(&db, "z-primary").await;
        assert_eq!(
            current.monitoring_baseline_initialized_at,
            initialized.monitoring_baseline_initialized_at
        );
        assert_eq!(
            current.last_check_attempt_at,
            initialized.last_check_attempt_at
        );
        assert_eq!(known(&db, "z-primary").await, history);
    }
    preference(&app, "z-primary", false).await;
    sweep(&db, &source, &queue, time() + TimeDelta::days(10)).await;
    let current = stored_artist(&db, "z-primary").await;
    assert_eq!(
        current.monitoring_baseline_initialized_at,
        initialized.monitoring_baseline_initialized_at
    );
    assert_eq!(
        current.last_check_attempt_at,
        Some((time() + TimeDelta::days(10)).fixed_offset())
    );
    assert_eq!(known(&db, "z-primary").await.len(), history.len() + 1);
    assert_eq!(catalog_rows(&db).await.0.len(), 1);
    no_jobs(&app).await;
    queue_worker.abort();
}

#[tokio::test]
async fn issue54_interrupted_attempt_retries_when_due_after_database_reopen() {
    let storage = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        storage.path().join("catalog.sqlite").display()
    );
    let db = Database::connect(&database_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let music = tempfile::tempdir().unwrap();
    let mut owned = ScanAlbum::new("owned", "Owned", "2024");
    owned.artists.retain(|artist| artist.id == "z-primary");
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![owned.clone()],
        discography_gates_by_artist_id: HashMap::from([(
            "z-primary".to_owned(),
            Arc::new(Semaphore::new(0)),
        )]),
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    let attempt = tokio::spawn({
        let db = db.clone();
        let source = source.clone();
        let queue = queue.clone();
        async move { sweep(&db, &source, &queue, time()).await }
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        source.discography_started.notified(),
    )
    .await
    .unwrap();
    let interrupted = stored_artist(&db, "z-primary").await;
    assert!(interrupted.monitoring_baseline_initialized_at.is_none());
    assert_eq!(
        interrupted.last_check_attempt_at,
        Some(time().fixed_offset())
    );
    assert!(known(&db, "z-primary").await.is_empty());
    attempt.abort();
    assert!(attempt.await.unwrap_err().is_cancelled());
    queue_worker.abort();
    let _ = queue_worker.await;
    drop(app);
    db.close().await.unwrap();

    let db = Database::connect(&database_url).await.unwrap();
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![owned],
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    assert_eq!(stored_artist(&db, "z-primary").await, interrupted);
    sweep(
        &db,
        &source,
        &queue,
        time() + TimeDelta::hours(6) - TimeDelta::seconds(1),
    )
    .await;
    assert_eq!(stored_artist(&db, "z-primary").await, interrupted);
    assert!(known(&db, "z-primary").await.is_empty());
    let retry_at = time() + TimeDelta::hours(6);
    sweep(&db, &source, &queue, retry_at).await;
    let initialized = stored_artist(&db, "z-primary").await;
    assert_eq!(
        initialized.monitoring_baseline_initialized_at,
        Some(retry_at.fixed_offset())
    );
    assert_eq!(
        initialized.last_check_attempt_at,
        Some(retry_at.fixed_offset())
    );
    assert_eq!(
        known(&db, "z-primary").await,
        vec![("owned".into(), "ALBUM".into())]
    );
    assert_eq!(catalog_rows(&db).await.0.len(), 1);
    no_jobs(&app).await;
    queue_worker.abort();
    let _ = queue_worker.await;
    drop(app);
    db.close().await.unwrap();
}

#[tokio::test]
async fn issue54_tidal_timeout_preserves_retry_timing_and_other_artists_initialize() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("owned", "Owned", "2024")],
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    source
        .discography_responses_by_artist_id
        .lock()
        .unwrap()
        .insert("a-guest".into(), Err(TidalError::ArtistAlbumsTimeout));
    sweep(&db, &source, &queue, time()).await;
    let timed_out = stored_artist(&db, "a-guest").await;
    assert!(timed_out.monitoring_baseline_initialized_at.is_none());
    assert_eq!(timed_out.last_check_attempt_at, Some(time().fixed_offset()));
    assert!(known(&db, "a-guest").await.is_empty());
    assert_eq!(
        known(&db, "z-primary").await,
        vec![("owned".into(), "ALBUM".into())]
    );

    sweep(
        &db,
        &source,
        &queue,
        time() + TimeDelta::hours(6) - TimeDelta::seconds(1),
    )
    .await;
    assert_eq!(stored_artist(&db, "a-guest").await, timed_out);
    sweep(&db, &source, &queue, time() + TimeDelta::hours(6)).await;
    assert!(
        stored_artist(&db, "a-guest")
            .await
            .monitoring_baseline_initialized_at
            .is_some()
    );
    assert_eq!(catalog_rows(&db).await.0.len(), 1);
    no_jobs(&app).await;
    queue_worker.abort();
}

#[tokio::test]
async fn issue54_preference_api_is_typed_persistent_and_does_not_create_artists() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("owned", "Owned", "2024")],
        ..Default::default()
    });
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    let first = preference(&app, "z-primary", true).await;
    assert_eq!(preference(&app, "z-primary", true).await, first);
    let reloaded = send_request(app.clone(), "GET", "/artists/z-primary", "").await;
    assert_eq!(reloaded.status(), StatusCode::OK);
    assert_eq!(json(reloaded).await, first);
    assert!(stored_artist(&db, "z-primary").await.monitored);
    assert!(!stored_artist(&db, "a-guest").await.monitored);
    for payload in [
        "",
        "{",
        "{}",
        r#"{"monitored":"true"}"#,
        r#"{"monitored":1}"#,
        r#"{"monitored":null}"#,
        r#"{"monitored":false,"extra":true}"#,
    ] {
        let response = send_request(app.clone(), "PATCH", "/artists/z-primary", payload).await;
        assert!(response.status().is_client_error(), "accepted {payload}");
        assert!(stored_artist(&db, "z-primary").await.monitored);
    }
    assert_eq!(
        send_request(
            app.clone(),
            "PATCH",
            "/artists/missing",
            r#"{"monitored":true}"#
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        artist::Entity::find_by_id("missing")
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
    preference(&app, "z-primary", false).await;
    assert!(!stored_artist(&db, "z-primary").await.monitored);
    no_jobs(&app).await;
    queue_worker.abort();
}

#[tokio::test]
async fn issue54_album_deletion_removes_only_orphan_artist_history() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let mut surviving = ScanAlbum::new("surviving", "Surviving", "2024");
    surviving.artists.retain(|artist| artist.id == "z-primary");
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("owned", "Owned", "2024"), surviving],
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    add(&app, "surviving").await;
    sweep(&db, &source, &queue, time()).await;
    let retained = known(&db, "z-primary").await;
    assert!(!known(&db, "a-guest").await.is_empty());
    delete(&app, "owned").await;
    assert!(
        artist::Entity::find_by_id("a-guest")
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
    assert!(known(&db, "a-guest").await.is_empty());
    assert_eq!(known(&db, "z-primary").await, retained);
    delete(&app, "surviving").await;
    assert!(
        artist_known_album::Entity::find()
            .all(&db)
            .await
            .unwrap()
            .is_empty()
    );
    queue_worker.abort();
}

#[tokio::test]
async fn issue54_monitoring_baseline_rolls_back_entries_and_completion_on_insert_failure() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    let mut owned = ScanAlbum::new("owned", "Owned", "2024");
    owned.artists.retain(|artist| artist.id == "z-primary");
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![owned],
        ..Default::default()
    });
    let (app, queue, queue_worker) = monitoring_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    observation(
        &source,
        "z-primary",
        vec![
            ScanAlbum::new("one", "One", "2000").album,
            ScanAlbum::new("two", "Two", "2000").album,
        ],
    );
    // A failure after the first insert must roll back the whole observation.
    db.execute_unprepared("CREATE TRIGGER fail_known_insert BEFORE INSERT ON artist_known_album WHEN (SELECT COUNT(*) FROM artist_known_album) >= 1 BEGIN SELECT RAISE(ABORT, 'test failure'); END").await.unwrap();
    sweep(&db, &source, &queue, time()).await;
    assert!(known(&db, "z-primary").await.is_empty());
    let failed = stored_artist(&db, "z-primary").await;
    assert!(failed.monitoring_baseline_initialized_at.is_none());
    assert_eq!(failed.last_check_attempt_at, Some(time().fixed_offset()));
    db.execute_unprepared("DROP TRIGGER fail_known_insert")
        .await
        .unwrap();
    sweep(&db, &source, &queue, time() + TimeDelta::hours(6)).await;
    assert!(
        stored_artist(&db, "z-primary")
            .await
            .monitoring_baseline_initialized_at
            .is_some()
    );
    assert_eq!(
        known(&db, "z-primary").await,
        vec![("owned".into(), "ALBUM".into())]
    );
    no_jobs(&app).await;
    queue_worker.abort();
}
