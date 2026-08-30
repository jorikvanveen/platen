use axum::{
    Router,
    routing::{delete, get, post},
};
use sea_orm::DatabaseConnection;

use crate::{
    routes,
    services::{download_queue::DownloadQueue, tidal::Tidal},
};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) tidal: Tidal,
    pub(crate) queue: DownloadQueue,
    pub(crate) db: DatabaseConnection,
}

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/artists", get(routes::artist::list))
        .route("/artists/{id}", get(routes::artist::get))
        .route(
            "/artists/{artist_id}/albums/{album_id}",
            post(routes::album::create_artist_scoped),
        )
        .route("/albums/{album_id}", post(routes::album::create))
        .route(
            "/catalog/refresh-artwork",
            post(routes::catalog::refresh_artwork),
        )
        .route(
            "/artists/{artist_id}/albums",
            get(routes::album::fetch_all_artist_albums),
        )
        .route(
            "/albums/refresh-release-dates",
            get(routes::album::refresh_release_dates),
        )
        .route("/albums/{album_id}/download", post(routes::album::download))
        .route("/downloads", get(routes::download::list))
        .route("/downloads/{job_id}", delete(routes::download::cancel))
        .route("/tidal/search/artists", get(routes::tidal::search_artists))
        .route("/tidal/search/albums", get(routes::tidal::search_albums))
        .route("/tidal/artists/{id}", get(routes::tidal::get_artist_albums))
        .route("/", get(|| async { "Hello world" }))
        .with_state(state)
}

#[cfg(test)]
mod tests {
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
    use sea_orm::{ActiveModelTrait, Database, Set};
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        entity::{album, album_artist, artist},
        services::downloaders::Downloader,
    };

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
            let mut maximum = self.max_active.load(Ordering::SeqCst);
            while active > maximum {
                match self.max_active.compare_exchange(
                    maximum,
                    active,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(current) => maximum = current,
                }
            }
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
            tidal: Tidal::new(String::new(), String::new()),
            queue,
            db,
        }
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
        let (queue, worker_handle) =
            DownloadQueue::start(db.clone(), PathBuf::from("/tmp/music"), downloader);
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
    async fn enqueue_rejects_when_worker_stopped_without_retaining_job() {
        let db = test_database().await;
        insert_test_album(&db, "album-1").await;
        let downloader = GateDownloader::new();
        let (queue, worker_handle) =
            DownloadQueue::start(db, PathBuf::from("/tmp/music"), downloader);
        worker_handle.abort();
        let _ = worker_handle.await;

        assert!(matches!(
            queue.enqueue("album-1".to_owned()).await,
            Err(crate::services::download_queue::QueueError::WorkerStopped)
        ));
        assert!(queue.active().await.is_empty());
    }

    #[tokio::test]
    async fn download_route_accepts_before_worker_finishes() {
        let db = test_database().await;
        insert_test_album(&db, "album-1").await;
        let downloader = GateDownloader::new();
        let (queue, worker_handle) =
            DownloadQueue::start(db.clone(), PathBuf::from("/tmp/music"), downloader.clone());
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
    async fn downloaded_album_conflict_is_returned_before_enqueueing() {
        let db = test_database().await;
        insert_test_album(&db, "album-1").await;
        album::ActiveModel {
            id: Set("album-1".to_owned()),
            downloaded: Set(true),
            ..Default::default()
        }
        .update(&db)
        .await
        .unwrap();
        let downloader = GateDownloader::new();
        let (queue, worker_handle) =
            DownloadQueue::start(db.clone(), PathBuf::from("/tmp/music"), downloader);
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
        assert!(queue.active().await.is_empty());
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
        let (queue, worker_handle) =
            DownloadQueue::start(db.clone(), PathBuf::from("/tmp/music"), downloader.clone());
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
        insert_test_album(&db, "album-running").await;
        let downloader = GateDownloader::new();
        let (queue, worker_handle) =
            DownloadQueue::start(db.clone(), PathBuf::from("/tmp/music"), downloader.clone());
        let app = router(app_state(db, queue.clone()));

        enqueue(&app, "album-running").await;
        tokio::time::timeout(Duration::from_secs(1), downloader.started.notified())
            .await
            .unwrap();
        let first_queued = enqueue(&app, "album-queued-0").await;
        for index in 1..1_000 {
            enqueue(&app, &format!("album-queued-{index}")).await;
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
        assert_eq!(queue.active().await.len(), 1_001);
        assert!(
            !queue
                .active()
                .await
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
        let (queue, worker_handle) =
            DownloadQueue::start(db.clone(), PathBuf::from("/tmp/music"), downloader.clone());
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
        let (queue, worker_handle) =
            DownloadQueue::start(db.clone(), PathBuf::from("/tmp/music"), downloader);
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
        let (queue, worker_handle) =
            DownloadQueue::start(db.clone(), PathBuf::from("/tmp/music"), downloader.clone());
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
        let (queue, worker_handle) =
            DownloadQueue::start(db.clone(), PathBuf::from("/tmp/music"), downloader.clone());
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
}
