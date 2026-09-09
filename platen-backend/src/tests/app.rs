use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{Router, body::Body, body::to_bytes, http::Request};
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, ConnectionTrait, Database, EntityTrait, QueryOrder, Set};
use tokio::sync::{Notify, Semaphore};
use tower::ServiceExt;

#[path = "album_deletion.rs"]
mod album_deletion;

use super::*;
use crate::{
    entity::{album, album_artist, artist},
    services::{
        downloaders::Downloader,
        import::ScanCoordinator,
        music_directory::MusicDirectory,
        rate_limit::RateLimit,
        tidal::{ResolvedTidalSearchedAlbum, TidalAlbum, TidalArtist, TidalCatalog, TidalError},
    },
    test_support::mocks::EmptyTidalCatalog,
};

#[derive(Clone)]
struct ScanAlbum {
    album: TidalAlbum,
    artists: Vec<TidalArtist>,
    failure: Option<&'static str>,
}

impl ScanAlbum {
    fn new(id: &str, title: &str, date: &str) -> Self {
        Self {
            album: TidalAlbum {
                id: id.to_owned(),
                title: title.to_owned(),
                cover_url: None,
                release_date: Some(date.to_owned()),
                popularity: 0.0,
                r#type: "ALBUM".to_owned(),
                explicit: None,
                media_tags: None,
            },
            artists: vec![
                TidalArtist {
                    id: "z-primary".to_owned(),
                    name: "Primary Artist".to_owned(),
                    profile_image_url: Some("https://example.test/primary.jpg".to_owned()),
                },
                TidalArtist {
                    id: "a-guest".to_owned(),
                    name: "Guest Artist".to_owned(),
                    profile_image_url: None,
                },
            ],
            failure: None,
        }
    }
}

#[derive(Default)]
struct FakeTidalCatalog {
    albums: Vec<ScanAlbum>,
    failed_searches: Vec<String>,
    search_gate: Option<Semaphore>,
    search_started: Notify,
    searches: StdMutex<Vec<String>>,
    metadata_calls: StdMutex<Vec<String>>,
}

impl FakeTidalCatalog {
    fn album(&self, id: &str) -> &ScanAlbum {
        self.albums
            .iter()
            .find(|record| record.album.id == id)
            .unwrap()
    }
}

#[async_trait::async_trait]
impl TidalCatalog for FakeTidalCatalog {
    async fn find_album(&self, query: &str) -> Result<Vec<ResolvedTidalSearchedAlbum>, TidalError> {
        self.searches.lock().unwrap().push(query.to_owned());
        self.search_started.notify_one();
        if let Some(gate) = &self.search_gate {
            gate.acquire().await.unwrap().forget();
        }
        if self.failed_searches.iter().any(|failed| failed == query) {
            return Err(TidalError::UnexpectedResponse);
        }
        Ok(self
            .albums
            .iter()
            .filter(|record| query == format!("Primary Artist {}", record.album.title))
            .map(|record| ResolvedTidalSearchedAlbum {
                id: record.album.id.clone(),
                title: record.album.title.clone(),
                cover_url: None,
                release_date: if record.failure == Some("date") {
                    Some("invalid".to_owned())
                } else {
                    record.album.release_date.clone()
                },
                popularity: 0.0,
                artists: if record.failure == Some("empty-credits") {
                    Vec::new()
                } else {
                    record.artists.clone()
                },
                r#type: record.album.r#type.clone(),
                explicit: record.album.explicit,
                media_tags: record.album.media_tags.clone(),
            })
            .collect())
    }

    async fn get_album(&self, id: &str) -> Result<TidalAlbum, TidalError> {
        self.metadata_calls.lock().unwrap().push(id.to_owned());
        let record = self.album(id);
        if record.failure == Some("metadata") {
            return Err(TidalError::UnexpectedResponse);
        }
        Ok(record.album.clone())
    }

    async fn get_album_cover(&self, id: &str) -> Result<Option<String>, TidalError> {
        self.metadata_calls.lock().unwrap().push(id.to_owned());
        if self.album(id).failure == Some("cover") {
            return Err(TidalError::UnexpectedResponse);
        }
        Ok(self.album(id).album.cover_url.clone())
    }

    async fn get_album_artists(&self, id: &str) -> Result<Vec<TidalArtist>, TidalError> {
        self.metadata_calls.lock().unwrap().push(id.to_owned());
        let record = self.album(id);
        if record.failure == Some("credits") {
            return Err(TidalError::UnexpectedResponse);
        }

        Ok(record.artists.clone())
    }
}

struct FailFirstDownloader {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl Downloader for FailFirstDownloader {
    async fn download_album(
        &self,
        _album: &album::Model,
        _destination: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(std::io::Error::other(
                "remote response exposed credential=secret and /private/music/path",
            )
            .into());
        }
        Ok(())
    }
}

struct GateLastDownloader {
    started: Notify,
    release: Notify,
}

#[async_trait::async_trait]
impl Downloader for GateLastDownloader {
    async fn download_album(
        &self,
        album: &album::Model,
        _destination: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if album.id == "album-active" {
            self.started.notify_one();
            self.release.notified().await;
        }
        Ok(())
    }
}

struct GateDownloader {
    started: Notify,
    release: Notify,
    active: AtomicUsize,
    max_active: AtomicUsize,
    album_starts: StdMutex<Vec<String>>,
}

impl GateDownloader {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            started: Notify::new(),
            release: Notify::new(),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            album_starts: StdMutex::new(Vec::new()),
        })
    }

    fn record_start(&self) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        self.started.notify_one();
    }
}

#[async_trait::async_trait]
impl Downloader for GateDownloader {
    async fn download_album(
        &self,
        album: &album::Model,
        _destination: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.album_starts.lock().unwrap().push(album.id.clone());
        self.record_start();
        self.release.notified().await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
}

async fn test_database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

async fn insert_test_album(db: &DatabaseConnection, id: &str) {
    album::ActiveModel {
        id: Set(id.to_owned()),
        title: Set(format!("Album {id}")),
        album_type: Set(Some("SINGLE".to_owned())),
        release_year: Set(2026),
        release_month: Set(None),
        release_day: Set(None),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
    artist::ActiveModel {
        id: Set(format!("artist-{id}")),
        name: Set("Test artist".to_owned()),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
    album_artist::ActiveModel {
        album_id: Set(id.to_owned()),
        artist_id: Set(format!("artist-{id}")),
        position: Set(0),
    }
    .insert(db)
    .await
    .unwrap();
}

fn app_state(db: DatabaseConnection, queue: DownloadQueue) -> AppState {
    AppState {
        tidal: Tidal::new(
            String::new(),
            String::new(),
            "NL".to_owned(),
            RateLimit::new(Duration::ZERO),
        ),
        queue,
        scan: ScanCoordinator::new(
            MusicDirectory::new(temp_music_dir()),
            db.clone(),
            Arc::new(EmptyTidalCatalog),
        ),
        db,
    }
}

fn temp_music_dir() -> PathBuf {
    tempfile::tempdir().unwrap().path().to_path_buf()
}

async fn enqueue(app: &Router, album_id: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/albums/{album_id}/download"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn downloads(app: &Router) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/downloads")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn scan_status(app: &Router) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/catalog/scan")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn run_scan(app: &Router) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/catalog/scan")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let started: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(started["phase"], "scanning");
    wait_for_scan(app).await
}

async fn wait_for_scan(app: &Router) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = scan_status(app).await;
            if status["phase"] == "completed" || status["phase"] == "failed" {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

async fn create_scan_audio(music_root: &Path, paths: &[&str]) {
    for path in paths {
        let disc = music_root.join(path).join("Disc 1");
        tokio::fs::create_dir_all(&disc).await.unwrap();
        tokio::fs::write(disc.join("track.OpUs"), b"audio")
            .await
            .unwrap();
    }
}

fn scan_app(
    db: &DatabaseConnection,
    music_root: &Path,
    source: Arc<FakeTidalCatalog>,
) -> (Router, tokio::task::JoinHandle<()>) {
    let music_directory = MusicDirectory::new(music_root.to_owned());
    let (queue, worker_handle) =
        DownloadQueue::start(db.clone(), music_directory.clone(), GateDownloader::new());
    let app = router(AppState {
        tidal: Tidal::new(
            String::new(),
            String::new(),
            "NL".to_owned(),
            RateLimit::new(Duration::ZERO),
        ),
        queue,
        scan: ScanCoordinator::new(music_directory, db.clone(), source),
        db: db.clone(),
    });
    (app, worker_handle)
}

async fn catalog_rows(
    db: &DatabaseConnection,
) -> (
    Vec<album::Model>,
    Vec<artist::Model>,
    Vec<album_artist::Model>,
) {
    (
        album::Entity::find()
            .order_by_asc(album::Column::Id)
            .all(db)
            .await
            .unwrap(),
        artist::Entity::find()
            .order_by_asc(artist::Column::Id)
            .all(db)
            .await
            .unwrap(),
        album_artist::Entity::find()
            .order_by_asc(album_artist::Column::AlbumId)
            .order_by_asc(album_artist::Column::Position)
            .all(db)
            .await
            .unwrap(),
    )
}

async fn wait_for_history(app: &Router, expected: usize) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let body = downloads(app).await;
            if body["history"].as_array().unwrap().len() == expected {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn catalog_artwork_refresh_route_is_available_via_post() {
    let db = test_database().await;
    let downloader = GateDownloader::new();
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader,
    );
    let app = router(app_state(db, queue));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/catalog/refresh-artwork")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "albums": {
                "updated": 0,
                "already_present": 0,
                "unavailable": 0,
                "failed": 0
            },
            "artists": {
                "updated": 0,
                "already_present": 0,
                "unavailable": 0,
                "failed": 0
            }
        })
    );
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn issue37_scan_returns_immediately_and_reports_matching_progress_and_conflict() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    create_scan_audio(
        music.path(),
        &[
            "Primary Artist/Alpha",
            "Primary Artist/Beta",
            "Primary Artist/Gamma",
        ],
    )
    .await;
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![
            ScanAlbum::new("alpha", "Alpha", "2024"),
            ScanAlbum::new("beta", "Beta", "2024"),
            ScanAlbum::new("gamma", "Gamma", "2024"),
        ],
        search_gate: Some(Semaphore::new(0)),
        ..Default::default()
    });
    let (app, worker_handle) = scan_app(&db, music.path(), source.clone());
    assert_eq!(scan_status(&app).await, serde_json::Value::Null);

    let response = tokio::time::timeout(
        Duration::from_secs(2),
        app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri("/catalog/scan")
                .body(Body::empty())
                .unwrap(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let started: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(started["phase"], "scanning");
    tokio::time::timeout(Duration::from_secs(2), source.search_started.notified())
        .await
        .unwrap();
    let matching = scan_status(&app).await;
    assert_eq!(matching["phase"], "matching");
    assert_eq!(matching["summary"]["album_directories_found"], 3);
    assert_eq!(matching["summary"]["candidates_total"], 3);
    assert_eq!(matching["summary"]["candidates_processed"], 0);
    assert!(catalog_rows(&db).await.0.is_empty());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/catalog/scan")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        matching
    );

    source.search_gate.as_ref().unwrap().add_permits(1);
    let progress = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = scan_status(&app).await;
            if status["summary"]["candidates_processed"] == 1 {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(progress["phase"], "matching");
    assert_eq!(progress["summary"]["candidates_total"], 3);
    assert_eq!(progress["summary"]["albums_imported"], 0);
    source.search_gate.as_ref().unwrap().add_permits(2);
    let completed = wait_for_scan(&app).await;
    assert_eq!(completed["phase"], "completed");
    assert_eq!(completed["summary"]["candidates_processed"], 3);
    assert_eq!(completed["summary"]["albums_imported"], 3);
    assert_eq!(completed["summary"]["failures"], 0);
    assert_eq!(source.searches.lock().unwrap().len(), 3);
    assert_eq!(scan_status(&app).await, completed);
    assert_eq!(catalog_rows(&db).await.0.len(), 3);
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn catalog_metadata_survives_creation_reads_and_repeated_adds_and_scans() {
    use serde_json::json;

    for explicit in [Some(true), Some(false), None] {
        for (tags, quality) in [
            (None, None),
            (Some(vec![]), None),
            (Some(vec!["FUTURE"]), None),
            (Some(vec!["LOSSLESS"]), Some("LOSSLESS")),
            (Some(vec!["HIRES_LOSSLESS"]), Some("HIRES_LOSSLESS")),
            (Some(vec!["FUTURE", "LOSSLESS"]), Some("LOSSLESS")),
            (Some(vec!["DOLBY_ATMOS"]), Some("DOLBY_ATMOS")),
            (
                Some(vec!["LOSSLESS", "DOLBY_ATMOS"]),
                Some("LOSSLESS + DOLBY_ATMOS"),
            ),
            (
                Some(vec!["LOSSLESS", "HIRES_LOSSLESS", "DOLBY_ATMOS", "FUTURE"]),
                Some("HIRES_LOSSLESS + DOLBY_ATMOS"),
            ),
        ] {
            for scan_created in [false, true] {
                let db = test_database().await;
                let music = tempfile::tempdir().unwrap();
                let mut record = ScanAlbum::new("edition", "Title", "2024");
                record.album.explicit = explicit;
                record.album.media_tags = tags
                    .as_ref()
                    .map(|tags| tags.iter().map(|tag| (*tag).to_owned()).collect());
                let source = Arc::new(FakeTidalCatalog {
                    albums: vec![record.clone()],
                    ..Default::default()
                });
                let (app, worker_handle) = scan_app(&db, music.path(), source.clone());

                if scan_created {
                    create_scan_audio(music.path(), &["Primary Artist/Title (2024)"]).await;
                    assert_eq!(run_scan(&app).await["summary"]["albums_imported"], 1);
                    assert!(source.metadata_calls.lock().unwrap().is_empty());
                } else {
                    let created =
                        crate::routes::album::create_with(&db, source.as_ref(), "edition")
                            .await
                            .unwrap();
                    let created = serde_json::to_value(created.0).unwrap();
                    assert_eq!(created["explicit"], json!(explicit));
                    assert_eq!(created["media_tags"], json!(tags));
                    assert_eq!(created["available_quality"], json!(quality));
                    assert_eq!(source.metadata_calls.lock().unwrap().len(), 3);
                }
                let stored = album::Entity::find_by_id("edition")
                    .one(&db)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(stored.explicit, explicit);
                assert_eq!(stored.media_tags, tags.clone().map(serde_json::Value::from));
                assert_eq!(
                    stored.relative_path.as_deref(),
                    scan_created.then_some("Primary Artist/Title (2024)")
                );

                record.album.explicit = Some(!explicit.unwrap_or(false));
                record.album.media_tags = Some(vec!["REPLACEMENT".into()]);
                let replacement = FakeTidalCatalog {
                    albums: vec![record],
                    ..Default::default()
                };
                let repeated = crate::routes::album::create_with(&db, &replacement, "edition")
                    .await
                    .unwrap();
                assert_eq!(repeated.0.explicit, explicit);
                assert_eq!(json!(repeated.0.media_tags), json!(tags));
                assert!(replacement.metadata_calls.lock().unwrap().is_empty());

                for (method, uri) in [
                    ("GET", "/artists/z-primary/albums"),
                    ("GET", "/artists/a-guest/albums"),
                    ("POST", "/albums/edition"),
                ] {
                    let response = app
                        .clone()
                        .oneshot(
                            Request::builder()
                                .method(method)
                                .uri(uri)
                                .body(Body::empty())
                                .unwrap(),
                        )
                        .await
                        .unwrap();
                    assert_eq!(response.status(), axum::http::StatusCode::OK);
                    let body: serde_json::Value = serde_json::from_slice(
                        &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
                    )
                    .unwrap();
                    let album = if method == "GET" {
                        assert_eq!(body.as_array().unwrap().len(), 1);
                        &body[0]
                    } else {
                        &body
                    };
                    assert_eq!(album["id"], "edition");
                    assert_eq!(album["explicit"], json!(explicit));
                    assert_eq!(album["media_tags"], json!(tags));
                    assert_eq!(album["available_quality"], json!(quality));
                    assert_eq!(album["artists"][0]["id"], "z-primary");
                    assert_eq!(album["artists"][1]["id"], "a-guest");
                }
                if scan_created {
                    assert_eq!(run_scan(&app).await["summary"]["albums_imported"], 0);
                    assert_eq!(source.searches.lock().unwrap().len(), 1);
                    assert!(source.metadata_calls.lock().unwrap().is_empty());
                }
                assert_eq!(
                    album::Entity::find_by_id("edition")
                        .one(&db)
                        .await
                        .unwrap()
                        .unwrap(),
                    stored
                );
                worker_handle.abort();
                let _ = worker_handle.await;
            }
        }
    }
}

#[tokio::test]
async fn issue37_scan_imports_unique_albums_and_preserves_known_metadata_on_rescan() {
    let db = test_database().await;
    insert_test_album(&db, "known").await;
    album::ActiveModel {
        id: Set("known".to_owned()),
        relative_path: Set(Some("Old/Location".to_owned())),
        release_month: Set(Some(6)),
        release_day: Set(Some(12)),
        cover_url: Set(Some("https://example.test/known.jpg".to_owned())),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    let before = catalog_rows(&db).await;
    let music = tempfile::tempdir().unwrap();
    create_scan_audio(
        music.path(),
        &[
            "Test artist/Album known (2026)",
            "Primary Artist/Collaboration (2024)",
            "Primary Artist/Undated folder",
        ],
    )
    .await;
    let collaboration = ScanAlbum::new("collaboration", "Collaboration", "2024-02-29");

    let source = Arc::new(FakeTidalCatalog {
        albums: vec![
            collaboration.clone(),
            collaboration,
            ScanAlbum::new("wrong-year", "Collaboration", "2025-02-28"),
            ScanAlbum::new("undated", "Undated folder", "2023"),
        ],
        ..Default::default()
    });
    let (app, worker_handle) = scan_app(&db, music.path(), source.clone());
    let completed = run_scan(&app).await;
    assert_eq!(completed["phase"], "completed");
    assert_eq!(completed["failure_reason"], serde_json::Value::Null);
    assert_eq!(
        completed["summary"],
        serde_json::json!({
            "album_directories_found": 3, "candidates_processed": 3, "candidates_total": 3,
            "albums_imported": 2, "locations_attached": 0, "locations_changed": 1,
            "unchanged_locations": 0, "locations_cleared": 0, "unmatched_candidates": 0,
            "ambiguous_matches": 0, "duplicate_locations": 0, "skipped_directories": 0,
            "failures": 0, "filesystem_errors": 0
        })
    );
    let after = catalog_rows(&db).await;
    assert_eq!(after.0.len(), 3);
    let mut known = before.0[0].clone();
    known.relative_path = Some("Test artist/Album known (2026)".to_owned());
    assert_eq!(
        after.0.iter().find(|album| album.id == "known"),
        Some(&known)
    );
    assert_eq!(
        after.1.iter().find(|artist| artist.id == "artist-known"),
        Some(&before.1[0])
    );
    assert_eq!(
        after.2.iter().find(|credit| credit.album_id == "known"),
        Some(&before.2[0])
    );
    for (id, path, year, month, day) in [
        (
            "collaboration",
            "Primary Artist/Collaboration (2024)",
            2024,
            Some(2),
            Some(29),
        ),
        ("undated", "Primary Artist/Undated folder", 2023, None, None),
    ] {
        let album = after.0.iter().find(|album| album.id == id).unwrap();
        assert_eq!(album.relative_path.as_deref(), Some(path));
        assert!(!Path::new(album.relative_path.as_ref().unwrap()).is_absolute());
        assert_eq!(
            (album.release_year, album.release_month, album.release_day),
            (year, month, day)
        );
        assert_eq!(album.album_type.as_deref(), Some("ALBUM"));
        assert_eq!(album.cover_url, None);
        let credits: Vec<_> = after
            .2
            .iter()
            .filter(|credit| credit.album_id == id)
            .map(|credit| (credit.artist_id.as_str(), credit.position))
            .collect();
        assert_eq!(credits, [("z-primary", 0), ("a-guest", 1)]);
    }
    let primary = after
        .1
        .iter()
        .find(|artist| artist.id == "z-primary")
        .unwrap();
    assert_eq!(primary.name, "Primary Artist");
    assert_eq!(
        primary.profile_image_url.as_deref(),
        Some("https://example.test/primary.jpg")
    );
    let guest = after
        .1
        .iter()
        .find(|artist| artist.id == "a-guest")
        .unwrap();
    assert_eq!(guest.name, "Guest Artist");
    assert_eq!(guest.profile_image_url, None);
    assert_eq!(after.1.len(), 3);
    assert_eq!(after.2.len(), 5);
    assert!(source.metadata_calls.lock().unwrap().is_empty());
    let mut searches = source.searches.lock().unwrap().clone();
    searches.sort();
    assert_eq!(
        searches,
        [
            "Primary Artist Collaboration",
            "Primary Artist Undated folder"
        ]
    );

    let second = run_scan(&app).await;
    let mut expected = completed;
    expected["summary"]["albums_imported"] = 0.into();
    expected["summary"]["locations_changed"] = 0.into();
    expected["summary"]["unchanged_locations"] = 3.into();
    assert_eq!(second, expected);
    assert_eq!(catalog_rows(&db).await, after);
    assert_eq!(source.searches.lock().unwrap().len(), 2);
    assert!(source.metadata_calls.lock().unwrap().is_empty());
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn issue37_scan_skips_ambiguity_duplicates_and_failures_without_losing_other_imports() {
    let db = test_database().await;
    db.execute_unprepared(
        "CREATE TRIGGER reject_scan_album BEFORE INSERT ON album \
         WHEN NEW.id = 'persistence' BEGIN SELECT RAISE(ABORT, 'test import failure'); END",
    )
    .await
    .unwrap();
    let music = tempfile::tempdir().unwrap();
    create_scan_audio(
        music.path(),
        &[
            "Primary Artist/Ambiguous",
            "Primary Artist/Duplicate",
            "Primary Artist/Duplicate (2024)",
            "Primary Artist/Search failure",
            "Primary Artist/Metadata failure",
            "Primary Artist/Credits failure",
            "Primary Artist/Cover failure",
            "Primary Artist/Invalid date",
            "Primary Artist/Empty credits",
            "Primary Artist/Persistence failure",
            "Primary Artist/No match",
            "Primary Artist/Good",
        ],
    )
    .await;
    let mut albums = vec![
        ScanAlbum::new("ambiguous-2024", "Ambiguous", "2024"),
        ScanAlbum::new("ambiguous-2025", "Ambiguous", "2025"),
        ScanAlbum::new("duplicate", "Duplicate", "2024"),
        ScanAlbum::new("good", "Good", "2024-06"),
        ScanAlbum::new("persistence", "Persistence failure", "2024"),
    ];
    for (id, title, failure) in [
        ("metadata", "Metadata failure", "metadata"),
        ("credits", "Credits failure", "credits"),
        ("cover", "Cover failure", "cover"),
        ("date", "Invalid date", "date"),
        ("empty", "Empty credits", "empty-credits"),
    ] {
        let mut record = ScanAlbum::new(id, title, "2024");
        record.failure = Some(failure);
        albums.push(record);
    }
    let source = Arc::new(FakeTidalCatalog {
        albums,
        failed_searches: vec!["Primary Artist Search failure".to_owned()],
        ..Default::default()
    });
    let (app, worker_handle) = scan_app(&db, music.path(), source.clone());
    let completed = run_scan(&app).await;
    assert_eq!(completed["phase"], "completed");
    assert_eq!(completed["failure_reason"], serde_json::Value::Null);
    assert_eq!(
        completed["summary"],
        serde_json::json!({
            "album_directories_found": 12, "candidates_processed": 12, "candidates_total": 12,
            "albums_imported": 4, "locations_attached": 0, "locations_changed": 0,
            "unchanged_locations": 0, "locations_cleared": 0, "unmatched_candidates": 1,
            "ambiguous_matches": 1, "duplicate_locations": 2, "skipped_directories": 8,
            "failures": 4, "filesystem_errors": 0
        })
    );
    let after = catalog_rows(&db).await;
    assert_eq!(after.0.len(), 4);
    for (id, path, month) in [
        ("cover", "Primary Artist/Cover failure", None),
        ("credits", "Primary Artist/Credits failure", None),
        ("good", "Primary Artist/Good", Some(6)),
        ("metadata", "Primary Artist/Metadata failure", None),
    ] {
        let album = after.0.iter().find(|album| album.id == id).unwrap();
        assert_eq!(album.cover_url, None);
        assert_eq!(album.relative_path.as_deref(), Some(path));
        assert_eq!(
            (album.release_year, album.release_month, album.release_day),
            (2024, month, None)
        );
        let credits: Vec<_> = after
            .2
            .iter()
            .filter(|credit| credit.album_id == id)
            .map(|credit| (credit.artist_id.as_str(), credit.position))
            .collect();
        assert_eq!(credits, [("z-primary", 0), ("a-guest", 1)]);
    }
    assert!(source.metadata_calls.lock().unwrap().is_empty());
    assert_eq!(after.1.len(), 2);
    assert_eq!(after.2.len(), 8);
    assert_eq!(source.searches.lock().unwrap().len(), 12);
    assert_eq!(scan_status(&app).await, completed);

    let second = run_scan(&app).await;
    let mut expected = completed;
    expected["summary"]["albums_imported"] = 0.into();
    expected["summary"]["unchanged_locations"] = 4.into();
    assert_eq!(second, expected);
    assert_eq!(catalog_rows(&db).await, after);
    assert_eq!(source.searches.lock().unwrap().len(), 20);
    assert!(source.metadata_calls.lock().unwrap().is_empty());
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn catalog_scan_runs_in_the_background_and_retains_its_summary() {
    let db = test_database().await;
    insert_test_album(&db, "locked").await;
    album::ActiveModel {
        id: Set("locked".to_owned()),
        title: Set("Album (With Notes)".to_owned()),
        release_year: Set(2025),
        relative_path: Set(Some("Old/Location".to_owned())),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    artist::ActiveModel {
        id: Set("artist-locked".to_owned()),
        name: Set("Primary Artist".to_owned()),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    let before = catalog_rows(&db).await;
    let music = tempfile::tempdir().unwrap();
    let album = music
        .path()
        .join("Primary Artist/Album (With Notes) (2025)/Disc 1");
    tokio::fs::create_dir_all(&album).await.unwrap();
    tokio::fs::write(album.join("track.OpUs"), b"audio")
        .await
        .unwrap();
    tokio::fs::create_dir_all(music.path().join("Primary Artist/Artwork only"))
        .await
        .unwrap();
    tokio::fs::write(
        music.path().join("Primary Artist/Artwork only/cover.jpg"),
        b"image",
    )
    .await
    .unwrap();

    let downloader = GateDownloader::new();
    let music_directory = MusicDirectory::new(music.path().to_owned());
    let guard = music_directory.lock().await;
    let (queue, worker_handle) =
        DownloadQueue::start(db.clone(), music_directory.clone(), downloader);
    let app = router(AppState {
        tidal: Tidal::new(
            String::new(),
            String::new(),
            "NL".to_owned(),
            RateLimit::new(Duration::ZERO),
        ),
        queue,
        scan: ScanCoordinator::new(
            music_directory.clone(),
            db.clone(),
            Arc::new(EmptyTidalCatalog),
        ),
        db: db.clone(),
    });

    assert_eq!(scan_status(&app).await, serde_json::Value::Null);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/catalog/scan")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/catalog/scan")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let active: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(active["phase"], "scanning");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(scan_status(&app).await["phase"], "scanning");
    assert_eq!(catalog_rows(&db).await, before);

    drop(guard);
    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = scan_status(&app).await;
            if status["phase"] == "completed" {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(completed["summary"]["album_directories_found"], 1);
    assert_eq!(completed["summary"]["candidates_processed"], 1);
    assert_eq!(completed["summary"]["candidates_total"], 1);
    assert_eq!(completed["summary"]["skipped_directories"], 1);
    assert_eq!(completed["summary"]["filesystem_errors"], 0);
    assert_eq!(completed["summary"]["locations_changed"], 1);
    let mut expected = before;
    expected.0[0].relative_path = Some("Primary Artist/Album (With Notes) (2025)".to_owned());
    assert_eq!(catalog_rows(&db).await, expected);

    assert_eq!(scan_status(&app).await, completed);
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn catalog_scan_reconciles_locations_without_changing_metadata_and_is_idempotent() {
    let db = test_database().await;
    let music = tempfile::tempdir().unwrap();
    for (id, path) in [
        ("attach", None),
        ("move", Some("Old/Move")),
        ("keep", Some("Unrelated artist/Old title (1999)")),
        ("clear", Some("Test artist/Album clear")),
        ("duplicate-new", None),
        ("duplicate-stale", Some("Old/Duplicate")),
        ("duplicate-kept", Some("Test artist/Album duplicate-kept")),
    ] {
        insert_test_album(&db, id).await;
        album::ActiveModel {
            id: Set(id.to_owned()),
            relative_path: Set(path.map(str::to_owned)),
            release_month: Set(Some(6)),
            release_day: Set(Some(12)),
            cover_url: Set(Some(format!("https://example.test/{id}/cover"))),
            ..Default::default()
        }
        .update(&db)
        .await
        .unwrap();
    }
    artist::ActiveModel {
        id: Set("a-guest".to_owned()),
        name: Set("Guest artist".to_owned()),
        profile_image_url: Set(Some("https://example.test/guest".to_owned())),
    }
    .insert(&db)
    .await
    .unwrap();
    album_artist::ActiveModel {
        album_id: Set("attach".to_owned()),
        artist_id: Set("a-guest".to_owned()),
        position: Set(1),
    }
    .insert(&db)
    .await
    .unwrap();
    for path in [
        "Test artist/Album attach (2026)",
        "Test artist/Album move",
        "Unrelated artist/Old title (1999)",
        "Test artist/Album duplicate-new",
        "Test artist/Album duplicate-new (2026)",
        "Test artist/Album duplicate-stale",
        "Test artist/Album duplicate-stale (2026)",
        "Test artist/Album duplicate-kept",
        "Test artist/Album duplicate-kept (2026)",
    ] {
        let disc = music.path().join(path).join("Disc 1");
        tokio::fs::create_dir_all(&disc).await.unwrap();
        tokio::fs::write(disc.join("track.OpUs"), b"audio")
            .await
            .unwrap();
    }
    let before = catalog_rows(&db).await;
    let music_directory = MusicDirectory::new(music.path().to_owned());
    let (queue, worker_handle) =
        DownloadQueue::start(db.clone(), music_directory.clone(), GateDownloader::new());
    let app = router(AppState {
        tidal: Tidal::new(
            String::new(),
            String::new(),
            "NL".to_owned(),
            RateLimit::new(Duration::ZERO),
        ),
        queue,
        scan: ScanCoordinator::new(music_directory, db.clone(), Arc::new(EmptyTidalCatalog)),
        db: db.clone(),
    });

    let completed = run_scan(&app).await;
    assert_eq!(completed["phase"], "completed");
    assert_eq!(completed["failure_reason"], serde_json::Value::Null);
    assert_eq!(
        completed["summary"],
        serde_json::json!({
            "album_directories_found": 9,
            "candidates_processed": 9,
            "candidates_total": 9,
            "albums_imported": 0,
            "locations_attached": 1,
            "locations_changed": 1,
            "unchanged_locations": 2,
            "locations_cleared": 2,
            "unmatched_candidates": 0,
            "ambiguous_matches": 0,
            "duplicate_locations": 6,
            "skipped_directories": 6,
            "failures": 0,
            "filesystem_errors": 0
        })
    );
    assert_eq!(scan_status(&app).await, completed);
    let after = catalog_rows(&db).await;
    let mut expected_albums = before.0;
    for album in &mut expected_albums {
        album.relative_path = match album.id.as_str() {
            "attach" => Some("Test artist/Album attach (2026)".to_owned()),
            "move" => Some("Test artist/Album move".to_owned()),
            "clear" | "duplicate-stale" => None,
            _ => album.relative_path.clone(),
        };
    }
    assert_eq!(after, (expected_albums, before.1, before.2));
    let attach_credits: Vec<_> = after
        .2
        .iter()
        .filter(|credit| credit.album_id == "attach")
        .map(|credit| (credit.artist_id.as_str(), credit.position))
        .collect();
    assert_eq!(attach_credits, [("artist-attach", 0), ("a-guest", 1)]);

    let second = run_scan(&app).await;
    let mut expected = completed;
    expected["summary"]["locations_attached"] = 0.into();
    expected["summary"]["locations_changed"] = 0.into();
    expected["summary"]["locations_cleared"] = 0.into();
    expected["summary"]["unchanged_locations"] = 4.into();
    assert_eq!(second, expected);
    assert_eq!(catalog_rows(&db).await, after);
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn catalog_scan_clears_paths_when_the_music_root_is_missing_even_with_failed_status() {
    let db = test_database().await;
    for id in ["missing", "unattached"] {
        insert_test_album(&db, id).await;
    }
    album::ActiveModel {
        id: Set("missing".to_owned()),
        relative_path: Set(Some("Test artist/Album missing".to_owned())),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    let before = catalog_rows(&db).await;
    let music = tempfile::tempdir().unwrap();
    let music_directory = MusicDirectory::new(music.path().join("missing-root"));
    let (queue, worker_handle) =
        DownloadQueue::start(db.clone(), music_directory.clone(), GateDownloader::new());
    let app = router(AppState {
        tidal: Tidal::new(
            String::new(),
            String::new(),
            "NL".to_owned(),
            RateLimit::new(Duration::ZERO),
        ),
        queue,
        scan: ScanCoordinator::new(music_directory, db.clone(), Arc::new(EmptyTidalCatalog)),
        db: db.clone(),
    });

    let failed = run_scan(&app).await;
    assert_eq!(failed["phase"], "failed");
    assert_eq!(
        failed["failure_reason"],
        "Could not scan the Music directory."
    );
    assert_eq!(failed["summary"]["locations_cleared"], 1);
    assert_eq!(failed["summary"]["filesystem_errors"], 1);
    assert_eq!(failed["summary"]["failures"], 1);
    assert_eq!(failed["summary"]["candidates_total"], 0);
    assert_eq!(failed["summary"]["candidates_processed"], 0);
    assert_eq!(scan_status(&app).await, failed);
    let mut expected = before;
    for album in &mut expected.0 {
        album.relative_path = None;
    }
    assert_eq!(catalog_rows(&db).await, expected);
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn enqueue_rejects_when_worker_stopped_without_retaining_job() {
    let db = test_database().await;
    insert_test_album(&db, "album-1").await;
    let downloader = GateDownloader::new();
    let (queue, worker_handle) =
        DownloadQueue::start(db, MusicDirectory::new(temp_music_dir()), downloader);
    worker_handle.abort();
    let _ = worker_handle.await;

    assert!(matches!(
        queue.enqueue("album-1".to_owned()).await,
        Err(crate::services::download_queue::QueueError::WorkerStopped)
    ));
    assert!(queue.snapshot().await.0.is_empty());
}

#[tokio::test]
async fn download_route_accepts_before_worker_finishes() {
    let db = test_database().await;
    insert_test_album(&db, "album-1").await;
    let downloader = GateDownloader::new();
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader.clone(),
    );
    let app = router(app_state(db, queue));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/albums/album-1/download")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let job: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(job["id"].as_str().is_some());
    assert_eq!(job["album_id"], "album-1");
    assert_eq!(job["release_name"], "Album album-1");
    assert_eq!(job["status"], "queued");
    chrono::DateTime::parse_from_rfc3339(job["enqueued_at"].as_str().unwrap()).unwrap();
    assert!(job["started_at"].is_null());
    assert!(job["finished_at"].is_null());

    tokio::time::timeout(Duration::from_secs(1), downloader.started.notified())
        .await
        .unwrap();
    assert_eq!(downloader.active.load(Ordering::SeqCst), 1);
    let body = downloads(&app).await;
    assert_eq!(body["active"][0]["status"], "running");
    chrono::DateTime::parse_from_rfc3339(body["active"][0]["started_at"].as_str().unwrap())
        .unwrap();
    assert!(body["active"][0]["finished_at"].is_null());
    downloader.release.notify_one();
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn download_responses_resolve_current_catalog_metadata() {
    use serde_json::json;

    for (explicit, media_tags, quality) in [
        (Some(true), Some(json!(["LOSSLESS"])), Some("LOSSLESS")),
        (
            Some(false),
            Some(json!(["HIRES_LOSSLESS"])),
            Some("HIRES_LOSSLESS"),
        ),
        (
            None,
            Some(json!(["LOSSLESS", "HIRES_LOSSLESS"])),
            Some("HIRES_LOSSLESS"),
        ),
        (
            Some(true),
            Some(json!(["FUTURE", "LOSSLESS"])),
            Some("LOSSLESS"),
        ),
        (
            Some(false),
            Some(json!(["DOLBY_ATMOS"])),
            Some("DOLBY_ATMOS"),
        ),
        (
            Some(true),
            Some(json!(["LOSSLESS", "DOLBY_ATMOS"])),
            Some("LOSSLESS + DOLBY_ATMOS"),
        ),
        (
            None,
            Some(json!([
                "DOLBY_ATMOS",
                "HIRES_LOSSLESS",
                "LOSSLESS",
                "FUTURE"
            ])),
            Some("HIRES_LOSSLESS + DOLBY_ATMOS"),
        ),
        (None, Some(json!(["FUTURE"])), None),
        (Some(false), Some(json!([])), None),
        (None, None, None),
    ] {
        let db = test_database().await;
        for album_id in ["album-1", "album-2"] {
            insert_test_album(&db, album_id).await;
            album::ActiveModel {
                id: Set(album_id.to_owned()),
                explicit: Set(explicit),
                media_tags: Set(media_tags.clone()),
                ..Default::default()
            }
            .update(&db)
            .await
            .unwrap();
        }
        let downloader = GateDownloader::new();
        let (queue, worker_handle) = DownloadQueue::start(
            db.clone(),
            MusicDirectory::new(temp_music_dir()),
            downloader.clone(),
        );
        let app = router(app_state(db.clone(), queue));
        let assert_metadata = |job: &serde_json::Value| {
            assert_eq!(job.get("explicit"), Some(&json!(explicit)));
            assert_eq!(job.get("available_quality"), Some(&json!(quality)));
            assert_eq!(
                job["release_name"],
                format!("Album {}", job["album_id"].as_str().unwrap())
            );
        };
        assert_metadata(&enqueue(&app, "album-1").await);
        tokio::time::timeout(Duration::from_secs(1), downloader.started.notified())
            .await
            .unwrap();
        let queued = enqueue(&app, "album-2").await;
        assert_metadata(&queued);
        let body = downloads(&app).await;
        assert_eq!(body["active"][0]["status"], "running");
        assert_eq!(body["active"][1]["status"], "queued");
        for job in body["active"].as_array().unwrap() {
            assert_metadata(job);
        }
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/downloads/{}", queued["id"].as_str().unwrap()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let cancelled: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_metadata(&cancelled);
        assert_eq!(cancelled["status"], "cancelled");
        downloader.release.notify_one();
        let body = wait_for_history(&app, 2).await;
        for job in body["history"].as_array().unwrap() {
            assert_metadata(job);
        }

        album::ActiveModel {
            id: Set("album-2".to_owned()),
            title: Set("Updated catalog title".to_owned()),
            explicit: Set(Some(true)),
            media_tags: Set(Some(json!(["HIRES_LOSSLESS", "DOLBY_ATMOS"]))),
            ..Default::default()
        }
        .update(&db)
        .await
        .unwrap();
        let body = downloads(&app).await;
        let updated = body["history"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["album_id"] == "album-2")
            .unwrap();
        assert_eq!(updated["release_name"], "Updated catalog title");
        assert_eq!(updated["explicit"], true);
        assert_eq!(updated["available_quality"], "HIRES_LOSSLESS + DOLBY_ATMOS");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/albums/album-2")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body = downloads(&app).await;
        let retained = body["history"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["album_id"] == "album-2")
            .unwrap();
        assert_eq!(retained["id"], queued["id"]);
        assert_eq!(retained["status"], "cancelled");
        for field in ["release_name", "explicit", "available_quality"] {
            assert_eq!(retained.get(field), Some(&json!(null)));
        }
        worker_handle.abort();
        let _ = worker_handle.await;
    }
}

#[tokio::test]
async fn downloaded_album_conflict_is_returned_before_enqueueing() {
    let db = test_database().await;
    insert_test_album(&db, "album-1").await;
    album::ActiveModel {
        id: Set("album-1".to_owned()),
        relative_path: Set(Some("Test artist/Album album-1 (2026)".to_owned())),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    let downloader = GateDownloader::new();
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader,
    );
    let app = router(app_state(db, queue.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/albums/album-1/download")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    assert!(queue.snapshot().await.0.is_empty());
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn unknown_album_is_rejected_before_enqueueing() {
    let db = test_database().await;
    let downloader = GateDownloader::new();
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader,
    );
    let app = router(app_state(db, queue.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/albums/unknown/download")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    assert!(queue.snapshot().await.0.is_empty());
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn duplicate_active_submission_returns_existing_job_without_reordering() {
    let db = test_database().await;
    for album_id in ["album-1", "album-2", "album-3"] {
        insert_test_album(&db, album_id).await;
    }
    let downloader = GateDownloader::new();
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader.clone(),
    );
    let app = router(app_state(db, queue));

    enqueue(&app, "album-1").await;
    tokio::time::timeout(Duration::from_secs(1), downloader.started.notified())
        .await
        .unwrap();
    let first_queued = enqueue(&app, "album-2").await;
    enqueue(&app, "album-3").await;
    let duplicate = enqueue(&app, "album-2").await;

    assert_eq!(duplicate["id"], first_queued["id"]);
    assert_eq!(duplicate["status"], "queued");
    let body = downloads(&app).await;
    let active = body["active"].as_array().unwrap();
    assert_eq!(active[0]["album_id"], "album-1");
    assert_eq!(active[1]["album_id"], "album-2");
    assert_eq!(active[2]["album_id"], "album-3");

    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn queue_accepts_one_thousand_waiting_jobs_and_rejects_the_next() {
    let db = test_database().await;
    for album_id in ["album-running", "album-queued-0", "album-over-limit"] {
        insert_test_album(&db, album_id).await;
    }
    let downloader = GateDownloader::new();
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader.clone(),
    );
    let app = router(app_state(db, queue.clone()));

    enqueue(&app, "album-running").await;
    tokio::time::timeout(Duration::from_secs(1), downloader.started.notified())
        .await
        .unwrap();
    let first_queued = enqueue(&app, "album-queued-0").await;
    for index in 1..1_000 {
        queue
            .enqueue(format!("album-queued-{index}"))
            .await
            .unwrap();
    }

    let duplicate = enqueue(&app, "album-queued-0").await;
    assert_eq!(duplicate["id"], first_queued["id"]);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/albums/album-over-limit/download")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(queue.snapshot().await.0.len(), 1_001);
    assert!(
        !queue
            .snapshot()
            .await
            .0
            .iter()
            .any(|job| job.album_id == "album-over-limit")
    );

    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn one_worker_does_not_run_downloads_concurrently() {
    let db = test_database().await;
    insert_test_album(&db, "album-1").await;
    insert_test_album(&db, "album-2").await;
    let downloader = GateDownloader::new();
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader.clone(),
    );
    let app = router(app_state(db, queue));

    for album_id in ["album-1", "album-2"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/albums/{album_id}/download"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    }
    tokio::time::timeout(Duration::from_secs(1), downloader.started.notified())
        .await
        .unwrap();
    assert_eq!(downloader.active.load(Ordering::SeqCst), 1);
    assert_eq!(
        downloader.album_starts.lock().unwrap().as_slice(),
        ["album-1"]
    );
    let body = downloads(&app).await;
    assert_eq!(body["active"][0]["album_id"], "album-1");
    assert_eq!(body["active"][0]["status"], "running");
    assert_eq!(body["active"][1]["album_id"], "album-2");
    assert_eq!(body["active"][1]["status"], "queued");

    downloader.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), downloader.started.notified())
        .await
        .unwrap();
    assert_eq!(downloader.max_active.load(Ordering::SeqCst), 1);
    assert_eq!(
        downloader.album_starts.lock().unwrap().as_slice(),
        ["album-1", "album-2"]
    );

    downloader.release.notify_one();
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn failed_download_is_safe_and_does_not_stall_later_work() {
    let db = test_database().await;
    insert_test_album(&db, "album-1").await;
    insert_test_album(&db, "album-2").await;
    let downloader = Arc::new(FailFirstDownloader {
        calls: AtomicUsize::new(0),
    });
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader,
    );
    let app = router(app_state(db, queue));

    enqueue(&app, "album-1").await;
    enqueue(&app, "album-2").await;
    let body = wait_for_history(&app, 2).await;

    assert!(body["active"].as_array().unwrap().is_empty());
    assert_eq!(body["history"][0]["album_id"], "album-2");
    assert_eq!(body["history"][0]["status"], "succeeded");
    assert_eq!(body["history"][1]["album_id"], "album-1");
    assert_eq!(body["history"][1]["status"], "failed");
    assert_eq!(
        body["history"][1]["failure_reason"],
        "Album download failed."
    );
    assert!(!body.to_string().contains("credential=secret"));
    assert!(!body.to_string().contains("/private/music/path"));

    for job in body["history"].as_array().unwrap() {
        chrono::DateTime::parse_from_rfc3339(job["enqueued_at"].as_str().unwrap()).unwrap();
        chrono::DateTime::parse_from_rfc3339(job["started_at"].as_str().unwrap()).unwrap();
        chrono::DateTime::parse_from_rfc3339(job["finished_at"].as_str().unwrap()).unwrap();
    }

    enqueue(&app, "album-1").await;
    let body = wait_for_history(&app, 3).await;
    assert_eq!(body["history"][0]["album_id"], "album-1");
    assert_eq!(body["history"][0]["status"], "succeeded");

    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn delete_cancels_queued_job_and_rejects_running_or_unknown_jobs() {
    let db = test_database().await;
    insert_test_album(&db, "album-1").await;
    insert_test_album(&db, "album-2").await;
    let downloader = GateDownloader::new();
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader.clone(),
    );
    let app = router(app_state(db, queue));

    let running = enqueue(&app, "album-1").await;
    tokio::time::timeout(Duration::from_secs(1), downloader.started.notified())
        .await
        .unwrap();
    let queued = enqueue(&app, "album-2").await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/downloads/{}", queued["id"].as_str().unwrap()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let cancelled: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(cancelled["id"], queued["id"]);
    assert_eq!(cancelled["album_id"], "album-2");
    assert_eq!(cancelled["release_name"], "Album album-2");
    assert_eq!(cancelled["status"], "cancelled");
    assert!(cancelled["started_at"].is_null());
    assert!(cancelled["finished_at"].is_string());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/downloads/{}", running["id"].as_str().unwrap()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(downloader.active.load(Ordering::SeqCst), 1);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/downloads/not-a-job")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    let history = downloads(&app).await;
    assert_eq!(history["history"][0]["status"], "cancelled");
    let retried = enqueue(&app, "album-2").await;
    assert_eq!(retried["status"], "queued");

    downloader.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), downloader.started.notified())
        .await
        .unwrap();
    downloader.release.notify_one();
    worker_handle.abort();
    let _ = worker_handle.await;
}

#[tokio::test]
async fn download_history_retains_latest_hundred_without_evicting_active_work() {
    let db = test_database().await;
    for index in 0..=100 {
        insert_test_album(&db, &format!("album-{index}")).await;
    }
    insert_test_album(&db, "album-active").await;
    let downloader = Arc::new(GateLastDownloader {
        started: Notify::new(),
        release: Notify::new(),
    });
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(temp_music_dir()),
        downloader.clone(),
    );
    let app = router(app_state(db, queue));

    for index in 0..=100 {
        enqueue(&app, &format!("album-{index}")).await;
    }
    enqueue(&app, "album-active").await;
    tokio::time::timeout(Duration::from_secs(5), downloader.started.notified())
        .await
        .unwrap();
    let body = downloads(&app).await;

    assert_eq!(body["active"].as_array().unwrap().len(), 1);
    assert_eq!(body["active"][0]["album_id"], "album-active");
    assert_eq!(body["active"][0]["status"], "running");
    assert_eq!(body["history"].as_array().unwrap().len(), 100);
    assert_eq!(body["history"][0]["album_id"], "album-100");
    assert_eq!(body["history"][99]["album_id"], "album-1");

    downloader.release.notify_one();
    worker_handle.abort();
    let _ = worker_handle.await;
}
