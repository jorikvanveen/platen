use super::*;
use reqwest::StatusCode;
use sea_orm::{ColumnTrait, QueryFilter, sea_query::Expr};
use serde_json::Value;
async fn summaries_with_query_count(
    app: &Router,
    query_count: &AtomicUsize,
) -> (HashMap<String, Value>, usize) {
    let previous = query_count.load(Ordering::SeqCst);
    let artists = summaries(app).await;
    (artists, query_count.load(Ordering::SeqCst) - previous)
}

async fn summaries(app: &Router) -> HashMap<String, Value> {
    let response = send_request(app.clone(), "GET", "/artists/summaries", "").await;
    assert_eq!(response.status(), StatusCode::OK);
    let artists: Vec<Value> =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    artists
        .into_iter()
        .map(|artist| {
            let total = artist["album_count"].as_i64().unwrap();
            let downloaded = artist["downloaded_album_count"].as_i64().unwrap();
            assert!(total >= 0 && downloaded >= 0 && downloaded <= total);
            assert_eq!(artist.as_object().unwrap().len(), 6);
            (artist["id"].as_str().unwrap().to_owned(), artist)
        })
        .collect()
}

fn assert_counts(artists: &HashMap<String, Value>, id: &str, total: i64, downloaded: i64) {
    assert_eq!(artists[id]["album_count"], total, "total for {id}");
    assert_eq!(
        artists[id]["downloaded_album_count"], downloaded,
        "downloaded for {id}"
    );
}

async fn set_location(db: &DatabaseConnection, id: &str, location: Option<&str>) {
    album::Entity::update_many()
        .col_expr(album::Column::RelativePath, Expr::value(location))
        .filter(album::Column::Id.eq(id))
        .exec(db)
        .await
        .unwrap();
}

#[tokio::test]
async fn artist_summaries_count_distinct_albums_for_every_credit_and_release_type() {
    let db = test_database().await;
    let root = tempfile::tempdir().unwrap();
    let shared = ScanAlbum::new("shared", "Same title", "2024");
    let mut ep = ScanAlbum::new("ep", "EP", "2024");
    ep.album.r#type = "EP".to_owned();
    ep.artists = vec![shared.artists[0].clone()];
    let mut single = ScanAlbum::new("single", "Single", "2024");
    single.album.r#type = "SINGLE".to_owned();
    single.artists = vec![shared.artists[1].clone()];
    let mut same_title = ScanAlbum::new("same-title", "Same title", "2024");
    same_title.artists = ep.artists.clone();
    let mut undownloaded = ScanAlbum::new("undownloaded", "Undownloaded", "2024");
    undownloaded.artists = vec![TidalArtist {
        id: "zero-downloads".to_owned(),
        name: "Zero downloads".to_owned(),
        profile_image_url: None,
    }];
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![shared, ep, single, same_title, undownloaded],
        ..Default::default()
    });
    let (app, worker) = scan_app(&db, root.path(), source).await;
    artist::ActiveModel {
        id: Set("retained".to_owned()),
        name: Set("Retained Artist".to_owned()),
        monitored: Set(true),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    for id in ["shared", "ep", "single", "same-title", "undownloaded"] {
        assert_eq!(
            send_request(app.clone(), "POST", &format!("/albums/{id}"), "")
                .await
                .status(),
            StatusCode::OK
        );
    }
    set_location(&db, "shared", Some("Primary Artist/Same title (2024)")).await;
    set_location(&db, "single", Some("Guest Artist/Single (2024)")).await;

    let artists = summaries(&app).await;
    assert_eq!(artists.len(), 4);
    assert_counts(&artists, "z-primary", 3, 1);
    assert_counts(&artists, "a-guest", 2, 2);
    assert_counts(&artists, "zero-downloads", 1, 0);
    assert_counts(&artists, "retained", 0, 0);
    assert_eq!(artists["retained"]["monitored"], true);
    assert_eq!(artists["z-primary"]["name"], "Primary Artist");
    assert_eq!(
        artists["z-primary"]["profile_image_url"],
        "https://example.test/primary.jpg"
    );
    assert_eq!(downloads(&app).await["history"], serde_json::json!([]));

    let response = send_request(
        app.clone(),
        "PATCH",
        "/artists/z-primary",
        r#"{"monitored":true}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let patched: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
    assert!(patched.get("album_count").is_none());
    let refreshed = summaries(&app).await;
    assert_counts(&refreshed, "z-primary", 3, 1);
    assert_eq!(refreshed["z-primary"]["monitored"], true);
    worker.abort();
}

#[tokio::test]
async fn artist_summaries_count_scanned_audio_without_download_history_and_refresh_cleared_locations()
 {
    let db = test_database().await;
    let root = tempfile::tempdir().unwrap();
    create_scan_audio(root.path(), &["Primary Artist/Scanned (2024)"]).await;
    let source = Arc::new(FakeTidalCatalog {
        albums: vec![ScanAlbum::new("scanned", "Scanned", "2024")],
        ..Default::default()
    });
    let (app, worker) = scan_app(&db, root.path(), source).await;
    run_scan(&app).await;
    let artists = summaries(&app).await;
    assert_counts(&artists, "z-primary", 1, 1);
    assert_counts(&artists, "a-guest", 1, 1);
    assert_eq!(downloads(&app).await["history"], serde_json::json!([]));

    tokio::fs::remove_dir_all(root.path().join("Primary Artist/Scanned (2024)"))
        .await
        .unwrap();
    run_scan(&app).await;
    let refreshed = summaries(&app).await;
    assert_counts(&refreshed, "z-primary", 1, 0);
    assert_counts(&refreshed, "a-guest", 1, 0);
    worker.abort();
}

#[tokio::test]
async fn artist_summaries_do_not_count_download_history_after_audio_location_is_cleared() {
    let db = test_database().await;
    let root = tempfile::tempdir().unwrap();
    insert_test_album(&db, "downloaded").await;
    let downloader = GateDownloader::new();
    let (queue, worker) = DownloadQueue::start(
        db.clone(),
        MusicDirectory::new(root.path().to_owned()),
        downloader.clone(),
    )
    .await
    .unwrap();
    let app = router(app_state(db.clone(), queue.clone()));
    queue.enqueue("downloaded".to_owned()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), downloader.started.notified())
        .await
        .unwrap();
    assert_counts(&summaries(&app).await, "artist-downloaded", 1, 0);
    downloader.release.notify_one();
    let history = wait_for_history(&app, 1).await;
    assert_eq!(history["history"][0]["status"], "succeeded");
    assert_counts(&summaries(&app).await, "artist-downloaded", 1, 1);

    set_location(&db, "downloaded", None).await;
    assert_counts(&summaries(&app).await, "artist-downloaded", 1, 0);
    assert_eq!(downloads(&app).await, history);
    worker.abort();
}

#[tokio::test]
async fn artist_summaries_return_empty_catalog_and_many_zero_credit_artists_without_tidal_requests()
{
    let db = test_database().await;
    let root = tempfile::tempdir().unwrap();
    let source = Arc::new(FakeTidalCatalog::default());
    let query_count = Arc::new(AtomicUsize::new(0));
    let counter = query_count.clone();
    let mut counted_db = db.clone();
    counted_db.set_metric_callback(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let (app, worker) = scan_app(&counted_db, root.path(), source.clone()).await;
    worker.abort();
    let _ = worker.await;
    let (empty_artists, empty_query_count) = summaries_with_query_count(&app, &query_count).await;
    assert!(empty_artists.is_empty());
    assert_eq!(empty_query_count, 1);
    for index in 0..200 {
        artist::ActiveModel {
            id: Set(format!("retained-{index}")),
            name: Set(format!("Artist {index}")),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
    }
    let (artists, large_catalog_query_count) = summaries_with_query_count(&app, &query_count).await;
    assert_eq!(artists.len(), 200);
    assert_eq!(large_catalog_query_count, 1);
    for index in 0..200 {
        assert_counts(&artists, &format!("retained-{index}"), 0, 0);
    }
    assert!(source.metadata_calls.lock().unwrap().is_empty());
    assert!(source.searches.lock().unwrap().is_empty());
    assert!(source.discography_calls.lock().unwrap().is_empty());
}
