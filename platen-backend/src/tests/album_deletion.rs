use super::*;
use reqwest::StatusCode;
use sea_orm::{ColumnTrait, QueryFilter};
use serde_json::{Value, json};

struct Fixture {
    root: tempfile::TempDir,
    db: DatabaseConnection,
    state: AppState,
    worker: tokio::task::JoinHandle<()>,
    downloader: Arc<GateDownloader>,
    directory: MusicDirectory,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let configured_root = root.path().to_owned();
        Self::with_root(root, configured_root).await
    }

    async fn with_root(root: tempfile::TempDir, configured_root: PathBuf) -> Self {
        let db = test_database().await;
        insert_test_album(&db, "selected").await;
        let directory = MusicDirectory::new(configured_root);
        let downloader = GateDownloader::new();
        let (queue, worker) =
            DownloadQueue::start(db.clone(), directory.clone(), downloader.clone());
        let state = AppState {
            tidal: Tidal::new(String::new(), String::new(), "NL".into()),
            queue,
            scan: ScanCoordinator::new(directory.clone(), db.clone(), Arc::new(EmptyTidalCatalog)),
            db: db.clone(),
        };
        Self {
            root,
            db,
            state,
            worker,
            downloader,
            directory,
        }
    }

    async fn location(&self, path: &str) {
        let mut album: album::ActiveModel = album::Entity::find_by_id("selected")
            .one(&self.db)
            .await
            .unwrap()
            .unwrap()
            .into();
        album.relative_path = Set(Some(path.to_owned()));
        album.update(&self.db).await.unwrap();
    }

    async fn request(&self, method: &str, path: &str, body: &str) -> (StatusCode, Value) {
        request(router(self.state.clone()), method, path, body).await
    }

    async fn delete(&self, body: &str) -> (StatusCode, Value) {
        self.request("DELETE", "/albums/selected", body).await
    }

    async fn exists(&self) -> bool {
        album::Entity::find_by_id("selected")
            .one(&self.db)
            .await
            .unwrap()
            .is_some()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

async fn request(app: Router, method: &str, path: &str, body: &str) -> (StatusCode, Value) {
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        app.oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        ),
    )
    .await
    .expect("request deadlocked")
    .unwrap();
    let status = response.status();
    let expected_content_type = if status.is_success() {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    };
    assert_eq!(response.headers()["content-type"], expected_content_type);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body = if status.is_success() {
        serde_json::from_slice(&bytes).unwrap()
    } else {
        Value::String(String::from_utf8(bytes.to_vec()).unwrap())
    };
    (status, body)
}

#[tokio::test]
async fn preview_includes_absolute_directory_even_when_files_are_absent() {
    let fixture = Fixture::new().await;
    let (status, preview) = fixture
        .request("GET", "/albums/selected/deletion-preview", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview, json!({"absolute_path": null}));

    fixture.location("Artist/Album").await;
    let (status, preview) = fixture
        .request("GET", "/albums/selected/deletion-preview", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        preview["absolute_path"],
        fixture.root.path().join("Artist/Album").to_str().unwrap()
    );
    assert!(fixture.exists().await);
}

#[tokio::test]
async fn preview_and_delete_only_remove_previously_credited_orphans() {
    let fixture = Fixture::new().await;
    insert_test_album(&fixture.db, "other").await;
    for id in ["guest", "unrelated"] {
        artist::ActiveModel {
            id: Set(id.to_owned()),
            name: Set(id.to_owned()),
            ..Default::default()
        }
        .insert(&fixture.db)
        .await
        .unwrap();
    }
    for (album_id, artist_id, position) in
        [("selected", "guest", 1), ("other", "artist-selected", 1)]
    {
        album_artist::ActiveModel {
            album_id: Set(album_id.to_owned()),
            artist_id: Set(artist_id.to_owned()),
            position: Set(position),
        }
        .insert(&fixture.db)
        .await
        .unwrap();
    }
    let (status, preview) = fixture
        .request("GET", "/albums/selected/deletion-preview", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview, json!({"absolute_path": null}));

    assert_eq!(
        fixture.delete("{}").await,
        (StatusCode::OK, json!({"removed_artist_ids": ["guest"]}))
    );
    assert!(!fixture.exists().await);
    for id in ["artist-selected", "artist-other", "unrelated"] {
        assert!(
            artist::Entity::find_by_id(id)
                .one(&fixture.db)
                .await
                .unwrap()
                .is_some()
        );
    }
    assert!(
        artist::Entity::find_by_id("guest")
            .one(&fixture.db)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        album_artist::Entity::find()
            .all(&fixture.db)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn catalog_only_preserves_files_and_allows_readding() {
    let fixture = Fixture::new().await;
    let album_path = fixture.root.path().join("Recorded/Actual");
    std::fs::create_dir_all(&album_path).unwrap();
    std::fs::write(album_path.join("cover.jpg"), "cover").unwrap();
    fixture.location("Recorded/Actual").await;
    assert_eq!(fixture.delete("{}").await.0, StatusCode::OK);
    assert!(album_path.join("cover.jpg").exists());
    insert_test_album(&fixture.db, "selected").await;
    fixture.location("Recorded/Actual").await;
    assert!(fixture.exists().await);
}

#[tokio::test]
async fn disk_delete_uses_recorded_location_and_removes_nonaudio_but_not_parent_or_siblings() {
    let fixture = Fixture::new().await;
    let album_path = fixture.root.path().join("Recorded/Actual");
    std::fs::create_dir_all(album_path.join("booklet")).unwrap();
    std::fs::create_dir_all(fixture.root.path().join("Recorded/Sibling")).unwrap();
    for name in ["track.flac", "cover.jpg", "booklet/notes.txt"] {
        std::fs::write(album_path.join(name), "content").unwrap();
    }
    fixture.location("Recorded/Actual").await;
    assert_eq!(
        fixture.delete(r#"{"delete_files":true}"#).await.0,
        StatusCode::OK
    );
    assert!(!album_path.exists());
    assert!(fixture.root.path().join("Recorded/Sibling").exists());
    assert!(!fixture.exists().await);
}

#[tokio::test]
async fn absent_directory_is_ok_but_absent_location_is_rejected() {
    let fixture = Fixture::new().await;
    assert_eq!(
        fixture.delete(r#"{"delete_files":true}"#).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(fixture.exists().await);
    fixture.location("Missing/Album").await;
    assert_eq!(
        fixture.delete(r#"{"delete_files":true}"#).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn unsafe_locations_retain_catalog_and_files() {
    let fixture = Fixture::new().await;
    std::fs::write(fixture.root.path().join("sentinel"), "keep").unwrap();
    for path in [
        "",
        ".",
        "..",
        "/tmp/album",
        "Artist",
        "Artist/..",
        "../Album",
        "Artist/./Album",
        "Artist//Album",
        "Artist/Album/",
        "Artist\\Album",
        "C:/Album",
        ".platen-staging/Album",
    ] {
        fixture.location(path).await;
        let (status, body) = fixture.delete(r#"{"delete_files":true}"#).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{path}");
        assert!(body.as_str().unwrap().contains("partially removed"));
        assert!(fixture.exists().await);
        assert!(fixture.root.path().join("sentinel").exists());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn configured_root_symlinks_and_symlink_ancestors_allow_deletion() {
    use std::os::unix::fs::symlink;
    for suffix in ["", "music", "./music", "music/."] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let actual_music_root = outside.path().join(suffix);
        let album_directory = actual_music_root.join("Artist/Album");
        std::fs::create_dir_all(&album_directory).unwrap();
        let sentinel = album_directory.join("track.flac");
        std::fs::write(&sentinel, "keep").unwrap();
        let link = root.path().join("link");
        symlink(outside.path(), &link).unwrap();
        let configured_root = link.join(suffix);
        let fixture = Fixture::with_root(root, configured_root).await;
        fixture.location("Artist/Album").await;

        let (status, _) = fixture.delete(r#"{"delete_files":true}"#).await;
        assert_eq!(status, StatusCode::OK, "{suffix}");
        assert!(!album_directory.exists());
        assert!(!fixture.exists().await);
    }
}

#[tokio::test]
async fn relative_configured_root_with_dot_components_can_delete_files() {
    let root = tempfile::tempdir_in(".").unwrap();
    let configured_root = Path::new(".")
        .join(root.path().file_name().unwrap())
        .join(".")
        .join("music")
        .join(".");
    let album_directory = root.path().join("music/Artist/Album");
    std::fs::create_dir_all(&album_directory).unwrap();
    std::fs::write(album_directory.join("track.flac"), "audio").unwrap();
    let fixture = Fixture::with_root(root, configured_root).await;
    fixture.location("Artist/Album").await;
    assert_eq!(
        fixture.delete(r#"{"delete_files":true}"#).await.0,
        StatusCode::OK
    );
    assert!(!album_directory.exists());
    assert!(!fixture.exists().await);
}

#[tokio::test]
async fn configured_dot_and_parent_components_are_supported_but_empty_root_is_rejected() {
    use crate::services::album_files::remove_album_directory;
    let directory = tempfile::tempdir_in(".").unwrap();
    let album_directory = directory.path().join("Album");
    let relative_path = format!(
        "{}/Album",
        directory.path().file_name().unwrap().to_str().unwrap()
    );
    for root in [Path::new(".").to_owned(), directory.path().join("..")] {
        std::fs::create_dir(&album_directory).unwrap();
        assert!(
            remove_album_directory(Path::new(""), &relative_path)
                .await
                .is_err()
        );
        assert!(album_directory.exists());
        assert!(remove_album_directory(&root, &relative_path).await.unwrap());
        assert!(!album_directory.exists());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn artist_symlinks_are_followed_but_album_and_nested_symlinks_only_remove_links() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("sentinel"), "keep").unwrap();
    for level in 0..3 {
        let fixture = Fixture::new().await;
        fixture.location("Artist/Album").await;
        let artist = fixture.root.path().join("Artist");
        let album = artist.join("Album");
        match level {
            0 => {
                std::fs::create_dir(outside.path().join("Album")).unwrap();
                std::fs::write(outside.path().join("Album/track.flac"), "audio").unwrap();
                symlink(outside.path(), &artist).unwrap();
            }
            1 => {
                std::fs::create_dir(&artist).unwrap();
                symlink(outside.path(), &album).unwrap();
            }
            _ => {
                std::fs::create_dir_all(&album).unwrap();
                symlink(outside.path(), album.join("linked")).unwrap();
            }
        }
        assert_eq!(
            fixture.delete(r#"{"delete_files":true}"#).await.0,
            StatusCode::OK
        );
        assert!(!fixture.exists().await);
        assert!(album.symlink_metadata().is_err());
        assert!(artist.symlink_metadata().is_ok());
        assert!(outside.path().join("sentinel").exists());
    }
}

#[tokio::test]
async fn database_failure_rolls_back_credits_and_reports_removed_files() {
    for disk in [false, true] {
        let fixture = Fixture::new().await;
        let album_path = fixture.root.path().join("Artist/Album");
        std::fs::create_dir_all(&album_path).unwrap();
        std::fs::write(album_path.join("cover.jpg"), "cover").unwrap();
        fixture.location("Artist/Album").await;
        fixture.db.execute_unprepared("CREATE TRIGGER fail_artist_delete BEFORE DELETE ON artist BEGIN SELECT RAISE(ABORT, 'test failure'); END").await.unwrap();
        let (status, body) = fixture
            .delete(&json!({"delete_files":disk}).to_string())
            .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(fixture.exists().await);
        assert_eq!(
            album_artist::Entity::find()
                .filter(album_artist::Column::AlbumId.eq("selected"))
                .all(&fixture.db)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(album_path.exists(), !disk);
        assert!(body.as_str().unwrap().contains(if disk {
            "files were removed, but catalog deletion failed"
        } else {
            "No files were removed"
        }));
    }
}

#[tokio::test]
async fn unknown_album_and_invalid_json_return_plain_text_errors() {
    let fixture = Fixture::new().await;
    for method in ["GET", "DELETE"] {
        let path = if method == "GET" {
            "/albums/unknown/deletion-preview"
        } else {
            "/albums/unknown"
        };
        let (status, body) = fixture.request(method, path, "{}").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.as_str().unwrap(), "Album not found.");
    }
    for body in ["{", r#"{"delete_files":"yes"}"#] {
        let (status, body) = fixture.delete(body).await;
        assert!(status.is_client_error());
        assert!(!body.as_str().unwrap().is_empty());
        assert!(fixture.exists().await);
    }
}

#[tokio::test]
async fn running_selected_download_does_not_reject_deletion() {
    let fixture = Fixture::new().await;
    fixture
        .state
        .queue
        .enqueue("selected".to_owned())
        .await
        .unwrap();
    fixture.downloader.started.notified().await;
    assert_eq!(fixture.delete("{}").await.0, StatusCode::OK);
    assert!(!fixture.exists().await);
    let (active, _) = fixture.state.queue.snapshot().await;
    assert!(active.iter().any(|job| job.album_id == "selected"
        && job.status == crate::services::download_queue::JobStatus::Running));
}

#[tokio::test]
async fn queued_selected_download_is_not_cancelled_by_deletion() {
    let fixture = Fixture::new().await;
    album::ActiveModel {
        id: Set("selected".to_owned()),
        explicit: Set(Some(true)),
        media_tags: Set(Some(json!(["HIRES_LOSSLESS", "DOLBY_ATMOS"]))),
        ..Default::default()
    }
    .update(&fixture.db)
    .await
    .unwrap();
    insert_test_album(&fixture.db, "other").await;
    fixture
        .state
        .queue
        .enqueue("other".to_owned())
        .await
        .unwrap();
    fixture.downloader.started.notified().await;
    fixture
        .state
        .queue
        .enqueue("selected".to_owned())
        .await
        .unwrap();
    assert_eq!(fixture.delete("{}").await.0, StatusCode::OK);
    assert!(!fixture.exists().await);
    let (active, history) = fixture.state.queue.snapshot().await;
    assert!(active.iter().any(|job| job.album_id == "selected"
        && job.status == crate::services::download_queue::JobStatus::Queued));
    assert!(active.iter().any(|job| job.album_id == "other"
        && job.status == crate::services::download_queue::JobStatus::Running));
    assert!(!history.iter().any(|job| job.album_id == "selected"));
    assert_eq!(*fixture.downloader.album_starts.lock().unwrap(), ["other"]);

    let (status, body) = fixture.request("GET", "/downloads", "").await;
    assert_eq!(status, StatusCode::OK);
    let selected = body["active"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["album_id"] == "selected")
        .unwrap();
    assert_eq!(selected["status"], "queued");
    for field in ["release_name", "explicit", "available_quality"] {
        assert_eq!(selected.get(field), Some(&Value::Null));
    }
    let (status, cancelled) = fixture
        .request(
            "DELETE",
            &format!("/downloads/{}", selected["id"].as_str().unwrap()),
            "",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["status"], "cancelled");
    for field in ["release_name", "explicit", "available_quality"] {
        assert_eq!(cancelled.get(field), Some(&Value::Null));
    }
}

#[tokio::test]
async fn active_scan_does_not_reject_deletion() {
    let fixture = Fixture::new().await;
    let _directory = fixture.directory.lock().await;
    fixture.state.scan.start().await.unwrap();
    assert_eq!(fixture.delete("{}").await.0, StatusCode::OK);
    assert!(!fixture.exists().await);
}

#[tokio::test]
async fn scan_can_reimport_album_deleted_during_matching() {
    let mut fixture = Fixture::new().await;
    create_scan_audio(fixture.root.path(), &["Primary Artist/Alpha (2026)"]).await;
    fixture.location("Primary Artist/Alpha (2026)").await;
    assert_eq!(fixture.delete("{}").await.0, StatusCode::OK);
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("selected", "Alpha", "2026")],
        search_gate: Some(Semaphore::new(0)),
        ..Default::default()
    });
    fixture.state.scan = ScanCoordinator::new(
        fixture.directory.clone(),
        fixture.db.clone(),
        source.clone(),
    );
    fixture.state.scan.start().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), source.search_started.notified())
        .await
        .unwrap();
    assert_eq!(
        fixture.state.scan.snapshot().await.unwrap().phase,
        crate::services::import::ScanPhase::Matching
    );
    insert_test_album(&fixture.db, "selected").await;
    assert_eq!(fixture.delete("{}").await.0, StatusCode::OK);
    assert!(!fixture.exists().await);
    source.search_gate.as_ref().unwrap().add_permits(1);
    assert_eq!(
        wait_for_scan(&router(fixture.state.clone())).await["phase"],
        "completed"
    );
    assert!(fixture.exists().await);
    assert_eq!(fixture.delete("{}").await.0, StatusCode::OK);
}

#[tokio::test]
async fn album_without_credits_does_not_remove_unrelated_orphans() {
    let fixture = Fixture::new().await;
    album_artist::Entity::delete_many()
        .exec(&fixture.db)
        .await
        .unwrap();
    let (status, preview) = fixture
        .request("GET", "/albums/selected/deletion-preview", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview, json!({"absolute_path": null}));
    assert_eq!(
        fixture.delete("{}").await,
        (StatusCode::OK, json!({"removed_artist_ids":[]}))
    );
    assert!(
        artist::Entity::find_by_id("artist-selected")
            .one(&fixture.db)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn missing_directory_does_not_claim_files_were_removed_when_catalog_fails() {
    let fixture = Fixture::new().await;
    fixture.location("Missing/Album").await;
    fixture.db.execute_unprepared("CREATE TRIGGER fail_album_delete BEFORE DELETE ON album BEGIN SELECT RAISE(ABORT, 'test failure'); END").await.unwrap();
    let (status, body) = fixture.delete(r#"{"delete_files":true}"#).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.as_str().unwrap().contains("No files were removed"));
    assert!(fixture.exists().await);
}
