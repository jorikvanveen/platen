use super::*;
use crate::{services::tidal, test_support::mocks::EmptyTidalCatalog};

fn test_scan() -> ActiveScan {
    ActiveScan::new().0
}

async fn discover(root: &Path) -> Vec<AlbumCandidate> {
    discover_album_candidates(root, &test_scan())
        .await
        .candidates
}

async fn summary(scan: &ActiveScan) -> ScanSummary {
    scan.status.snapshot.lock().await.summary.clone()
}

#[tokio::test]
async fn discovery_owns_counts_and_publishes_progress_without_reading_it_back() {
    let music = tempfile::tempdir().unwrap();
    let album = music.path().join("Artist/Title (2024)");
    tokio::fs::create_dir_all(&album).await.unwrap();
    tokio::fs::write(album.join("track.flac"), b"audio")
        .await
        .unwrap();
    tokio::fs::create_dir_all(music.path().join("Artist/Empty"))
        .await
        .unwrap();
    let scan = test_scan();
    scan.publish_summary(&ScanSummary {
        album_directories_found: 99,
        skipped_directories: 99,
        locations_attached: 99,
        ..Default::default()
    })
    .await;

    let report = discover_album_candidates(music.path(), &scan).await;
    let discovered = ScanSummary::from(&report);
    assert_eq!(
        discovered,
        ScanSummary {
            album_directories_found: 1,
            candidates_total: 1,
            skipped_directories: 1,
            ..Default::default()
        }
    );
    let progress = summary(&scan).await;
    assert_eq!(progress.album_directories_found, 1);
    assert_eq!(progress.skipped_directories, 1);

    scan.publish_summary(&discovered).await;
    assert_eq!(summary(&scan).await, discovered);
}

#[test]
fn discovery_summary_includes_root_failure_diagnostics() {
    let report = DiscoveryReport {
        diagnostics: vec![FilesystemDiagnostic {
            reason: "resolve_music_root",
            path: "missing".into(),
            os_error: std::io::ErrorKind::NotFound.into(),
        }],
        root_failed: true,
        ..Default::default()
    };
    assert_eq!(
        ScanSummary::from(&report),
        ScanSummary {
            filesystem_errors: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn discovers_supported_audio_recursively_and_parses_only_the_final_year() {
    let music = tempfile::tempdir().unwrap();
    let album = music.path().join("Artist/Title (Live) (2024)/Disc 1");
    tokio::fs::create_dir_all(&album).await.unwrap();
    tokio::fs::write(album.join("track.FLAC"), b"audio")
        .await
        .unwrap();

    let candidates = discover(music.path()).await;

    assert_eq!(
        candidates,
        vec![AlbumCandidate {
            primary_artist: "Artist".to_owned(),
            title: "Title (Live)".to_owned(),
            release_year: Some(2024),
            relative_path: "Artist/Title (Live) (2024)".to_owned(),
        }]
    );
}

#[tokio::test]
async fn accepts_every_supported_extension_case_insensitively() {
    let music = tempfile::tempdir().unwrap();
    for (index, extension) in [
        "flac", "MP3", "M4a", "aac", "OGG", "opus", "WAV", "aiff", "AIF", "alac",
    ]
    .into_iter()
    .enumerate()
    {
        let album = music.path().join(format!("Artist/Album {index}"));
        tokio::fs::create_dir_all(&album).await.unwrap();
        tokio::fs::write(album.join(format!("track.{extension}")), b"audio")
            .await
            .unwrap();
    }

    assert_eq!(discover(music.path()).await.len(), 10);
}

#[tokio::test]
async fn skips_empty_artwork_only_malformed_and_staging_directories() {
    let music = tempfile::tempdir().unwrap();
    for album in ["Empty", "Artwork", "(2024)"] {
        tokio::fs::create_dir_all(music.path().join("Artist").join(album))
            .await
            .unwrap();
    }
    tokio::fs::write(music.path().join("Artist/Artwork/cover.jpg"), b"image")
        .await
        .unwrap();
    let staging = music.path().join(STAGING_DIRECTORY).join("job");
    tokio::fs::create_dir_all(&staging).await.unwrap();
    tokio::fs::write(staging.join("track.flac"), b"audio")
        .await
        .unwrap();

    let scan = test_scan();
    let report = discover_album_candidates(music.path(), &scan).await;

    assert!(report.candidates.is_empty());
    assert_eq!(summary(&scan).await.skipped_directories, 3);
}

#[cfg(unix)]
#[tokio::test]
async fn resolves_a_symbolic_link_root_but_does_not_follow_descendants() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().unwrap();
    let real = parent.path().join("real");
    tokio::fs::create_dir_all(real.join("Artist/Real album"))
        .await
        .unwrap();
    tokio::fs::write(real.join("Artist/Real album/track.flac"), b"audio")
        .await
        .unwrap();
    let outside = parent.path().join("outside/Linked album");
    tokio::fs::create_dir_all(&outside).await.unwrap();
    tokio::fs::write(outside.join("track.mp3"), b"audio")
        .await
        .unwrap();
    symlink(&outside, real.join("Artist/Linked album")).unwrap();
    let root_link = parent.path().join("music");
    symlink(&real, &root_link).unwrap();

    let scan = test_scan();
    let candidates = discover_album_candidates(&root_link, &scan)
        .await
        .candidates;

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].title, "Real album");
    assert_eq!(summary(&scan).await.skipped_directories, 1);
}

#[cfg(unix)]
#[tokio::test]
async fn skips_non_utf8_directories_without_panicking() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let music = tempfile::tempdir().unwrap();
    let invalid = OsString::from_vec(vec![b'A', 0xff]);
    tokio::fs::create_dir_all(music.path().join(invalid))
        .await
        .unwrap();

    let scan = test_scan();
    let report = discover_album_candidates(music.path(), &scan).await;

    assert!(report.candidates.is_empty());
    assert_eq!(summary(&scan).await.skipped_directories, 1);
}

#[tokio::test]
async fn a_missing_root_is_a_terminal_failure() {
    let root = tempfile::tempdir().unwrap().path().join("missing");
    let scan = test_scan();

    let report = discover_album_candidates(&root, &scan).await;
    assert!(report.root_failed);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].path, root);
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
    let scan = test_scan();
    let report = discover_album_candidates(&root, &scan).await;
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
    let (scan, handle, _) = ActiveScan::new();
    coordinator.run(scan).await;
    let snapshot = handle.snapshot().await;
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
