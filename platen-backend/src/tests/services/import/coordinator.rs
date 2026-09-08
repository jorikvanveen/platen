use super::super::{discovery::discover_album_candidates, model::ScanPhase};
use super::*;
use crate::{services::tidal, test_support::mocks::EmptyTidalCatalog};

async fn terminal_snapshot(coordinator: &ScanCoordinator) -> ScanSnapshot {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let snapshot = coordinator.snapshot().await.unwrap();
            if !snapshot.phase.is_active() {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_panicked_worker_reports_failure_and_allows_another_scan() {
    let root = tempfile::tempdir().unwrap();
    let album_path = root.path().join("Artist/Title");
    tokio::fs::create_dir_all(&album_path).await.unwrap();
    tokio::fs::write(album_path.join("track.flac"), b"audio")
        .await
        .unwrap();
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    <migration::Migrator as migration::MigratorTrait>::up(&db, None)
        .await
        .unwrap();
    let source = Arc::new(GatedCatalog {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
    });
    let coordinator =
        ScanCoordinator::new(MusicDirectory::new(root.path().into()), db, source.clone());
    coordinator.start().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), source.entered.notified())
        .await
        .unwrap();
    source.release.close();
    let failed = terminal_snapshot(&coordinator).await;
    assert_eq!(failed.phase, ScanPhase::Failed);
    assert_eq!(failed.summary.candidates_total, 1);
    assert_eq!(failed.summary.failures, 1);
    assert_eq!(
        failed.failure_reason.as_deref(),
        Some("Music directory scan stopped unexpectedly.")
    );
    assert_eq!(coordinator.snapshot().await, Some(failed));
    tokio::fs::remove_dir_all(album_path).await.unwrap();
    coordinator.start().await.unwrap();
    assert_eq!(
        terminal_snapshot(&coordinator).await.phase,
        ScanPhase::Completed
    );
}

#[tokio::test]
async fn concurrent_start_requests_admit_exactly_one_worker() {
    let root = tempfile::tempdir().unwrap();
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    <migration::Migrator as migration::MigratorTrait>::up(&db, None)
        .await
        .unwrap();
    let music = MusicDirectory::new(root.path().into());
    let guard = music.lock().await;
    let coordinator = ScanCoordinator::new(music.clone(), db, Arc::new(EmptyTidalCatalog));
    let starts = futures_util::future::join_all((0..20).map(|_| coordinator.start())).await;
    assert_eq!(starts.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(starts.iter().filter(|result| result.is_err()).count(), 19);
    drop(guard);
    assert_eq!(
        terminal_snapshot(&coordinator).await.phase,
        ScanPhase::Completed
    );
}

struct GatedCatalog {
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}

#[async_trait::async_trait]
impl TidalCatalog for GatedCatalog {
    async fn find_album(
        &self,
        _: &str,
    ) -> Result<Vec<tidal::ResolvedTidalSearchedAlbum>, tidal::TidalError> {
        self.entered.notify_one();
        self.release.acquire().await.unwrap().forget();
        Ok(vec![])
    }

    async fn get_album(&self, id: &str) -> Result<tidal::TidalAlbum, tidal::TidalError> {
        EmptyTidalCatalog.get_album(id).await
    }

    async fn get_album_cover(&self, id: &str) -> Result<Option<String>, tidal::TidalError> {
        EmptyTidalCatalog.get_album_cover(id).await
    }

    async fn get_album_artists(
        &self,
        id: &str,
    ) -> Result<Vec<tidal::TidalArtist>, tidal::TidalError> {
        EmptyTidalCatalog.get_album_artists(id).await
    }
}

#[tokio::test]
async fn matching_releases_music_lock_and_remains_observable_until_completion() {
    let root = tempfile::tempdir().unwrap();
    let album_path = root.path().join("Artist/Title");
    tokio::fs::create_dir_all(&album_path).await.unwrap();
    tokio::fs::write(album_path.join("track.flac"), b"audio")
        .await
        .unwrap();
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    <migration::Migrator as migration::MigratorTrait>::up(&db, None)
        .await
        .unwrap();
    let music = MusicDirectory::new(root.path().into());
    let source = Arc::new(GatedCatalog {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
    });
    let coordinator = ScanCoordinator::new(music.clone(), db, source.clone());
    coordinator.start().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), source.entered.notified())
        .await
        .unwrap();
    let snapshot = coordinator.snapshot().await.unwrap();
    assert_eq!(snapshot.phase, ScanPhase::Matching);
    assert_eq!(snapshot.summary.candidates_total, 1);
    assert_eq!(snapshot.summary.candidates_processed, 0);
    assert_eq!(snapshot.summary.unmatched_candidates, 0);
    assert!(coordinator.start().await.is_err());
    let _guard = tokio::time::timeout(std::time::Duration::from_secs(1), music.lock())
        .await
        .unwrap();
    source.release.add_permits(1);
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let snapshot = coordinator.snapshot().await.unwrap();
            if !snapshot.phase.is_active() {
                break snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(terminal.phase, ScanPhase::Completed);
    assert_eq!(terminal.summary.candidates_processed, 1);
    assert_eq!(terminal.summary.unmatched_candidates, 1);
    assert_eq!(terminal.summary.skipped_directories, 1);
}

#[tokio::test]
async fn coordinator_waits_for_music_lock_and_completes_after_reconciliation() {
    use crate::entity::{album, album_artist, artist};
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let root = tempfile::tempdir().unwrap();
    let location = root.path().join("Artist/Title (2024)");
    tokio::fs::create_dir_all(&location).await.unwrap();
    tokio::fs::write(location.join("track.flac"), b"audio")
        .await
        .unwrap();
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    <migration::Migrator as migration::MigratorTrait>::up(&db, None)
        .await
        .unwrap();
    album::ActiveModel {
        id: Set("album".into()),
        title: Set("Title".into()),
        release_year: Set(2024),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    artist::ActiveModel {
        id: Set("artist".into()),
        name: Set("Artist".into()),
        profile_image_url: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();
    album_artist::ActiveModel {
        album_id: Set("album".into()),
        artist_id: Set("artist".into()),
        position: Set(0),
    }
    .insert(&db)
    .await
    .unwrap();
    let music = MusicDirectory::new(root.path().to_owned());
    let guard = music.lock().await;
    let coordinator = ScanCoordinator::new(music.clone(), db.clone(), Arc::new(EmptyTidalCatalog));
    coordinator.start().await.unwrap();
    assert!(coordinator.start().await.is_err());
    tokio::task::yield_now().await;
    assert_eq!(
        coordinator
            .snapshot()
            .await
            .unwrap()
            .summary
            .album_directories_found,
        0
    );
    assert_eq!(
        album::Entity::find_by_id("album")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .relative_path,
        None
    );
    drop(guard);
    let completed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let snapshot = coordinator.snapshot().await.unwrap();
            if !snapshot.phase.is_active() {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(completed.phase, ScanPhase::Completed);
    assert_eq!(completed.summary.locations_attached, 1);
    assert_eq!(completed.summary.candidates_processed, 1);
    assert_eq!(
        album::Entity::find_by_id("album")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .relative_path
            .as_deref(),
        Some("Artist/Title (2024)")
    );
    let _guard = tokio::time::timeout(std::time::Duration::from_secs(1), music.lock())
        .await
        .unwrap();
}

#[tokio::test]
async fn unreadable_root_report_clears_locations_before_failure() {
    use crate::entity::album;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("not-a-directory");
    tokio::fs::write(&root, b"file").await.unwrap();
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    <migration::Migrator as migration::MigratorTrait>::up(&db, None)
        .await
        .unwrap();
    album::ActiveModel {
        id: Set("stale".into()),
        title: Set("Stale".into()),
        release_year: Set(2024),
        relative_path: Set(Some("Artist/Stale".into())),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let report = discover_album_candidates(&root, |_| {}).await;
    assert!(report.root_failed);
    assert_eq!(
        report.diagnostics[0].path,
        tokio::fs::canonicalize(&root).await.unwrap()
    );
    let coordinator = ScanCoordinator::new(
        MusicDirectory::new(root),
        db.clone(),
        Arc::new(EmptyTidalCatalog),
    );
    let snapshot = coordinator.workflow.run(|_| {}).await;
    assert_eq!(snapshot.phase, ScanPhase::Failed);
    assert_eq!(snapshot.summary.locations_cleared, 1);
    assert_eq!(snapshot.summary.filesystem_errors, 1);
    assert_eq!(
        album::Entity::find_by_id("stale")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .relative_path,
        None
    );
}

#[tokio::test]
async fn coordinator_retains_a_terminal_failure() {
    let root = tempfile::tempdir().unwrap().path().join("missing");
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    <migration::Migrator as migration::MigratorTrait>::up(&db, None)
        .await
        .unwrap();
    use crate::entity::album;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    album::ActiveModel {
        id: Set("stale".into()),
        title: Set("Stale".into()),
        release_year: Set(2024),
        relative_path: Set(Some("Artist/Stale".into())),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let music_directory = MusicDirectory::new(root);
    let coordinator = ScanCoordinator::new(
        music_directory.clone(),
        db.clone(),
        Arc::new(EmptyTidalCatalog),
    );

    assert_eq!(
        coordinator.start().await.unwrap().phase,
        ScanPhase::Scanning
    );
    let failed = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let snapshot = coordinator.snapshot().await.unwrap();
            if snapshot.phase == ScanPhase::Failed {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    assert_eq!(failed.summary.failures, 1);
    assert_eq!(failed.summary.filesystem_errors, 1);
    assert_eq!(failed.summary.locations_cleared, 1);
    assert_eq!(
        album::Entity::find_by_id("stale")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .relative_path,
        None
    );
    assert_eq!(coordinator.snapshot().await, Some(failed));
    let _guard = tokio::time::timeout(std::time::Duration::from_secs(1), music_directory.lock())
        .await
        .unwrap();
}
