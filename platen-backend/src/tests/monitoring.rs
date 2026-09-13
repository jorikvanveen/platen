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

async fn sweep(db: &DatabaseConnection, source: &FakeTidalCatalog, now: DateTime<Utc>) {
    monitor::initialize_due_artists(db, source, now)
        .await
        .unwrap();
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
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    for id in ["a-guest", "z-primary"] {
        let artist = stored_artist(&db, id).await;
        assert!(!artist.monitored);
        assert!(artist.monitoring_baseline_initialized_at.is_none());
        assert!(artist.last_check_attempt_at.is_none());
    }
    let worker = monitor::start(db.clone(), source.clone());
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
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
    assert_eq!(run_scan(&app).await["phase"], "completed");
    for id in ["a-guest", "z-primary"] {
        assert!(!stored_artist(&db, id).await.monitored);
        observation(&source, id, vec![]);
    }
    sweep(&db, &source, time()).await;
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
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
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
    sweep(&db, &source, time()).await;
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
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    source
        .discography_responses_by_artist_id
        .lock()
        .unwrap()
        .insert("z-primary".into(), Err(TidalError::UnexpectedResponse));
    sweep(&db, &source, time()).await;
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
        time() + TimeDelta::hours(6) - TimeDelta::seconds(1),
    )
    .await;
    assert_eq!(stored_artist(&db, "z-primary").await, failed);
    sweep(&db, &source, time() + TimeDelta::hours(6)).await;
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
        sweep(&db, &source, time() + TimeDelta::days(10)).await;
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
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    let attempt = tokio::spawn({
        let db = db.clone();
        let source = source.clone();
        async move { sweep(&db, &source, time()).await }
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
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
    assert_eq!(stored_artist(&db, "z-primary").await, interrupted);
    sweep(
        &db,
        &source,
        time() + TimeDelta::hours(6) - TimeDelta::seconds(1),
    )
    .await;
    assert_eq!(stored_artist(&db, "z-primary").await, interrupted);
    assert!(known(&db, "z-primary").await.is_empty());
    let retry_at = time() + TimeDelta::hours(6);
    sweep(&db, &source, retry_at).await;
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
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    source
        .discography_responses_by_artist_id
        .lock()
        .unwrap()
        .insert("a-guest".into(), Err(TidalError::ArtistAlbumsTimeout));
    sweep(&db, &source, time()).await;
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
        time() + TimeDelta::hours(6) - TimeDelta::seconds(1),
    )
    .await;
    assert_eq!(stored_artist(&db, "a-guest").await, timed_out);
    sweep(&db, &source, time() + TimeDelta::hours(6)).await;
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
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
    add(&app, "owned").await;
    add(&app, "surviving").await;
    sweep(&db, &source, time()).await;
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
    let (app, queue_worker) = scan_app(&db, music.path(), source.clone());
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
    sweep(&db, &source, time()).await;
    assert!(known(&db, "z-primary").await.is_empty());
    let failed = stored_artist(&db, "z-primary").await;
    assert!(failed.monitoring_baseline_initialized_at.is_none());
    assert_eq!(failed.last_check_attempt_at, Some(time().fixed_offset()));
    db.execute_unprepared("DROP TRIGGER fail_known_insert")
        .await
        .unwrap();
    sweep(&db, &source, time() + TimeDelta::hours(6)).await;
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
