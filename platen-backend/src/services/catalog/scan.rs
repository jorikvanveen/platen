use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;

use super::{matching::match_and_import, reconciliation::reconcile};

use crate::{
    routes::album::STAGING_DIRECTORY,
    services::{music_directory::MusicDirectory, tidal::TidalCatalog},
};

#[cfg(test)]
use crate::services::tidal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanPhase {
    Scanning,
    Matching,
    Completed,
    Failed,
}

impl ScanPhase {
    pub(crate) fn is_active(self) -> bool {
        matches!(self, Self::Scanning | Self::Matching)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanSummary {
    pub(crate) album_directories_found: usize,
    pub(crate) candidates_processed: usize,
    pub(crate) candidates_total: usize,
    pub(crate) albums_imported: usize,
    pub(crate) locations_attached: usize,
    pub(crate) locations_changed: usize,
    pub(crate) unchanged_locations: usize,
    pub(crate) locations_cleared: usize,
    pub(crate) unmatched_candidates: usize,
    pub(crate) ambiguous_matches: usize,
    pub(crate) duplicate_locations: usize,
    pub(crate) skipped_directories: usize,
    pub(crate) failures: usize,
    pub(crate) filesystem_errors: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanSnapshot {
    pub(crate) phase: ScanPhase,
    pub(crate) summary: ScanSummary,
    pub(crate) failure_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AlbumCandidate {
    pub(crate) primary_artist: String,
    pub(crate) title: String,
    pub(crate) release_year: Option<i32>,
    pub(crate) relative_path: String,
}

#[derive(Default)]
pub(super) struct DiscoveryReport {
    pub(super) resolved_root: Option<PathBuf>,
    pub(super) reconciled_duplicate_paths: HashSet<String>,
    pub(super) reconciled_duplicate_album_ids: HashSet<String>,
    pub(super) candidates: Vec<AlbumCandidate>,
    pub(super) diagnostics: Vec<FilesystemDiagnostic>,
    pub(super) root_failed: bool,
    pub(super) skipped_directories: usize,
}

impl DiscoveryReport {
    pub(super) fn absolute_path(&self, relative_path: &str) -> PathBuf {
        self.resolved_root
            .as_deref()
            .unwrap_or(Path::new(""))
            .join(relative_path)
    }
}

impl From<&DiscoveryReport> for ScanSummary {
    fn from(report: &DiscoveryReport) -> Self {
        Self {
            album_directories_found: report.candidates.len(),
            candidates_total: report.candidates.len(),
            filesystem_errors: report.diagnostics.len(),
            skipped_directories: report.skipped_directories,
            ..Default::default()
        }
    }
}

pub(super) struct FilesystemDiagnostic {
    pub(super) reason: &'static str,
    pub(super) path: PathBuf,
    pub(super) os_error: std::io::Error,
}

impl FilesystemDiagnostic {
    fn log(&self) {
        tracing::error!(
            reason = self.reason,
            path = %self.path.display(),
            os_error = %self.os_error,
            "Filesystem error during Music scan"
        );
    }
}

enum ScannedEntryKind {
    Directory(String),
    File,
    Other,
}

struct ScanStatus {
    snapshot: Mutex<ScanSnapshot>,
}

#[derive(Clone)]
pub(super) struct ScanHandle {
    status: Arc<ScanStatus>,
}

pub(super) struct ActiveScan {
    status: Arc<ScanStatus>,
}

impl ScanHandle {
    pub(super) async fn snapshot(&self) -> ScanSnapshot {
        self.status.snapshot.lock().await.clone()
    }
}

impl ActiveScan {
    pub(super) fn new() -> (Self, ScanHandle, ScanSnapshot) {
        let snapshot = ScanSnapshot {
            phase: ScanPhase::Scanning,
            summary: ScanSummary::default(),
            failure_reason: None,
        };
        let status = Arc::new(ScanStatus {
            snapshot: Mutex::new(snapshot.clone()),
        });

        (
            Self {
                status: status.clone(),
            },
            ScanHandle { status },
            snapshot,
        )
    }

    pub(super) async fn publish_summary(&self, summary: &ScanSummary) {
        self.status.snapshot.lock().await.summary = summary.clone();
    }

    async fn complete(self, candidate_count: usize) {
        let mut snapshot = self.status.snapshot.lock().await;
        snapshot.phase = ScanPhase::Completed;
        snapshot.summary.candidates_processed = candidate_count;
        snapshot.summary.candidates_total = candidate_count;
    }

    async fn fail(self, reason: &str) {
        let mut snapshot = self.status.snapshot.lock().await;
        snapshot.phase = ScanPhase::Failed;
        snapshot.summary.failures = snapshot.summary.failures.saturating_add(1);
        snapshot.failure_reason = Some(reason.to_owned());
    }
}

#[derive(Clone)]
pub(crate) struct ScanCoordinator {
    music_directory: MusicDirectory,
    db: DatabaseConnection,
    tidal: Arc<dyn TidalCatalog>,
    latest_scan: Arc<Mutex<Option<ScanHandle>>>,
}

impl ScanCoordinator {
    pub(crate) fn new(
        music_directory: MusicDirectory,
        db: DatabaseConnection,
        tidal: Arc<dyn TidalCatalog>,
    ) -> Self {
        Self {
            music_directory,
            db,
            tidal,
            latest_scan: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) async fn start(&self) -> Result<ScanSnapshot, ScanSnapshot> {
        let (active_scan, snapshot) = {
            let mut latest_scan = self.latest_scan.lock().await;
            if let Some(scan) = latest_scan.as_ref() {
                let snapshot = scan.snapshot().await;
                if snapshot.phase.is_active() {
                    return Err(snapshot);
                }
            }

            let (active_scan, handle, snapshot) = ActiveScan::new();
            *latest_scan = Some(handle);
            (active_scan, snapshot)
        };

        let coordinator = self.clone();
        tokio::spawn(coordinator.run(active_scan));
        Ok(snapshot)
    }

    pub(crate) async fn snapshot(&self) -> Option<ScanSnapshot> {
        Some(self.latest_scan.lock().await.as_ref()?.snapshot().await)
    }

    async fn run(self, scan: ActiveScan) {
        let (report, mut summary, result) = {
            let _music_dir_guard = self.music_directory.lock().await;
            let mut report = discover_album_candidates(self.music_directory.path(), &scan).await;
            let mut summary = ScanSummary::from(&report);
            scan.publish_summary(&summary).await;
            let result = reconcile(&self.db, &mut report, &mut summary).await;
            scan.publish_summary(&summary).await;
            (report, summary, result)
        };

        match result {
            Err(error) => {
                let path = report
                    .resolved_root
                    .as_deref()
                    .unwrap_or(self.music_directory.path());
                tracing::error!(reason = "catalog_reconciliation", path = %path.display(), %error, "Could not reconcile Catalog locations");
                scan.fail("Could not reconcile Catalog locations.").await;
            }
            Ok(_) if report.root_failed => {
                scan.fail("Could not scan the Music directory.").await;
            }
            Ok(candidates) => {
                scan.status.snapshot.lock().await.phase = ScanPhase::Matching;
                match_and_import(
                    &self.db,
                    self.tidal.as_ref(),
                    &report,
                    candidates,
                    &mut summary,
                    &scan,
                )
                .await;
                scan.publish_summary(&summary).await;
                scan.complete(report.candidates.len()).await;
            }
        }
    }
}

async fn discover_album_candidates(configured_root: &Path, scan: &ActiveScan) -> DiscoveryReport {
    let music_root = match tokio::fs::canonicalize(configured_root).await {
        Ok(root) => root,
        Err(error) => {
            let diagnostic = FilesystemDiagnostic {
                reason: "resolve_music_root",
                path: configured_root.to_owned(),
                os_error: error,
            };
            diagnostic.log();
            return DiscoveryReport {
                diagnostics: vec![diagnostic],
                root_failed: true,
                ..Default::default()
            };
        }
    };
    let mut scanner = Scanner {
        music_root,
        scan,
        diagnostics: Vec::new(),
        skipped_directories: 0,
    };
    let result = scanner.discover().await;
    DiscoveryReport {
        resolved_root: Some(scanner.music_root),
        root_failed: result.is_err(),
        candidates: result.unwrap_or_default(),
        diagnostics: scanner.diagnostics,
        skipped_directories: scanner.skipped_directories,
        ..Default::default()
    }
}

struct Scanner<'a> {
    music_root: PathBuf,
    scan: &'a ActiveScan,
    diagnostics: Vec<FilesystemDiagnostic>,
    skipped_directories: usize,
}

impl Scanner<'_> {
    async fn discover(&mut self) -> Result<Vec<AlbumCandidate>, ()> {
        let music_root = self.music_root.clone();
        let artist_entries = self.list_directory_entries(&music_root).await.ok_or(())?;
        let mut candidates = Vec::new();

        for artist_entry in artist_entries {
            if artist_entry.file_name() == STAGING_DIRECTORY {
                continue;
            }
            let Some(ScannedEntryKind::Directory(artist_name)) =
                self.classify_entry(&artist_entry).await
            else {
                continue;
            };
            let Some(album_entries) = self.list_directory_entries(&artist_entry.path()).await
            else {
                self.record_skipped("unreadable_artist_directory", &artist_entry.path())
                    .await;
                continue;
            };

            for album_entry in album_entries {
                if album_entry.file_name() == STAGING_DIRECTORY {
                    continue;
                }
                let Some(ScannedEntryKind::Directory(album_name)) =
                    self.classify_entry(&album_entry).await
                else {
                    continue;
                };
                let Some((title, release_year)) = parse_album_directory_name(&album_name) else {
                    self.record_skipped("malformed_album_directory", &album_entry.path())
                        .await;
                    continue;
                };
                if !self.contains_audio_file(&album_entry.path()).await {
                    self.record_skipped("no_supported_audio", &album_entry.path())
                        .await;
                    continue;
                }

                let relative_path = format!("{artist_name}/{album_name}");
                candidates.push(AlbumCandidate {
                    primary_artist: artist_name.clone(),
                    title,
                    release_year,
                    relative_path,
                });
                let mut snapshot = self.scan.status.snapshot.lock().await;
                snapshot.summary.album_directories_found = candidates.len();
            }
        }

        Ok(candidates)
    }

    async fn list_directory_entries(
        &mut self,
        absolute_path: &Path,
    ) -> Option<Vec<tokio::fs::DirEntry>> {
        let mut directory = match tokio::fs::read_dir(absolute_path).await {
            Ok(directory) => directory,
            Err(error) => {
                self.record_filesystem_error("read_directory", absolute_path, error);
                return None;
            }
        };
        let mut entries = Vec::new();
        loop {
            match directory.next_entry().await {
                Ok(Some(entry)) => entries.push(entry),
                Ok(None) => return Some(entries),
                Err(error) => {
                    self.record_filesystem_error("read_directory_entry", absolute_path, error);
                    return Some(entries);
                }
            }
        }
    }

    async fn classify_entry(&mut self, entry: &tokio::fs::DirEntry) -> Option<ScannedEntryKind> {
        let file_type = match entry.file_type().await {
            Ok(file_type) => file_type,
            Err(error) => {
                self.record_filesystem_error("read_entry_type", &entry.path(), error);
                return None;
            }
        };
        if file_type.is_symlink() {
            self.record_skipped("symbolic_link", &entry.path()).await;
            return None;
        }
        if file_type.is_dir() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                self.record_skipped("non_utf8_directory", &entry.path())
                    .await;
                return None;
            };
            return Some(ScannedEntryKind::Directory(name));
        }
        if file_type.is_file() {
            return Some(ScannedEntryKind::File);
        }
        Some(ScannedEntryKind::Other)
    }

    async fn contains_audio_file(&mut self, album_root: &Path) -> bool {
        let mut pending = VecDeque::from([album_root.to_owned()]);
        let mut found_audio = false;
        while let Some(absolute) = pending.pop_front() {
            let Some(entries) = self.list_directory_entries(&absolute).await else {
                continue;
            };
            for entry in entries {
                if entry.file_name() == STAGING_DIRECTORY {
                    continue;
                }
                match self.classify_entry(&entry).await {
                    Some(ScannedEntryKind::Directory(_)) => {
                        pending.push_back(entry.path());
                    }
                    Some(ScannedEntryKind::File) if is_audio_file(&entry.path()) => {
                        found_audio = true;
                    }
                    _ => {}
                }
            }
        }
        found_audio
    }

    fn record_filesystem_error(
        &mut self,
        reason: &'static str,
        path: &Path,
        os_error: std::io::Error,
    ) {
        let diagnostic = FilesystemDiagnostic {
            reason,
            path: path.to_owned(),
            os_error,
        };
        diagnostic.log();
        self.diagnostics.push(diagnostic);
    }

    async fn record_skipped(&mut self, reason: &'static str, path: &Path) {
        self.skipped_directories = self.skipped_directories.saturating_add(1);
        {
            let mut snapshot = self.scan.status.snapshot.lock().await;
            snapshot.summary.skipped_directories = self.skipped_directories;
        }
        tracing::warn!(
            reason,
            path = %path.display(),
            "Skipped directory during Music scan"
        );
    }
}

fn parse_album_directory_name(name: &str) -> Option<(String, Option<i32>)> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(without_close) = trimmed.strip_suffix(')')
        && let Some((title, year)) = without_close.rsplit_once(" (")
        && year.len() == 4
        && year.chars().all(|character| character.is_ascii_digit())
    {
        let title = title.trim();
        if title.is_empty() {
            return None;
        }
        return Some((title.to_owned(), year.parse().ok()));
    }
    Some((trimmed.to_owned(), None))
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "flac" | "mp3" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "aiff" | "aif" | "alac"
            )
        })
}

#[cfg(test)]
pub(crate) struct EmptyTidalCatalog;

#[cfg(test)]
#[async_trait::async_trait]
impl TidalCatalog for EmptyTidalCatalog {
    async fn find_album(
        &self,
        _: &str,
    ) -> Result<Vec<tidal::ResolvedTidalSearchedAlbum>, tidal::TidalError> {
        Ok(Vec::new())
    }

    async fn get_album(&self, _: &str) -> Result<tidal::TidalAlbum, tidal::TidalError> {
        Err(tidal::TidalError::UnexpectedResponse)
    }

    async fn get_album_cover(&self, _: &str) -> Result<Option<String>, tidal::TidalError> {
        Err(tidal::TidalError::UnexpectedResponse)
    }

    async fn get_album_artists(
        &self,
        _: &str,
    ) -> Result<Vec<tidal::TidalArtist>, tidal::TidalError> {
        Err(tidal::TidalError::UnexpectedResponse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let coordinator =
            ScanCoordinator::new(music.clone(), db.clone(), Arc::new(EmptyTidalCatalog));
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
        let _guard =
            tokio::time::timeout(std::time::Duration::from_secs(1), music_directory.lock())
                .await
                .unwrap();
    }
}
