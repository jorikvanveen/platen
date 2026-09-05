use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::sync::Mutex;

use crate::{routes::album::STAGING_DIRECTORY, services::music_directory::MusicDirectory};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanPhase {
    Scanning,
    #[allow(dead_code)]
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
    pub(crate) album_directories_found: u32,
    pub(crate) candidates_processed: u32,
    pub(crate) candidates_total: u32,
    pub(crate) albums_imported: u32,
    pub(crate) locations_attached: u32,
    pub(crate) locations_changed: u32,
    pub(crate) unchanged_locations: u32,
    pub(crate) locations_cleared: u32,
    pub(crate) unmatched_candidates: u32,
    pub(crate) ambiguous_matches: u32,
    pub(crate) duplicate_locations: u32,
    pub(crate) skipped_directories: u32,
    pub(crate) failures: u32,
    pub(crate) filesystem_errors: u32,
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

enum ScannedEntryKind {
    Directory(String),
    File,
    Other,
}

struct ScanStatus {
    snapshot: Mutex<ScanSnapshot>,
}

#[derive(Clone)]
struct ScanHandle {
    status: Arc<ScanStatus>,
}

struct ActiveScan {
    status: Arc<ScanStatus>,
}

impl ScanHandle {
    async fn snapshot(&self) -> ScanSnapshot {
        self.status.snapshot.lock().await.clone()
    }
}

impl ActiveScan {
    fn new() -> (Self, ScanHandle, ScanSnapshot) {
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

    async fn complete(self, candidate_count: u32) {
        let mut snapshot = self.status.snapshot.lock().await;
        snapshot.phase = ScanPhase::Completed;
        snapshot.summary.candidates_processed = candidate_count;
        snapshot.summary.candidates_total = candidate_count;
    }

    async fn fail(self, reason: &str) {
        let mut snapshot = self.status.snapshot.lock().await;
        snapshot.phase = ScanPhase::Failed;
        snapshot.summary.failures = 1;
        snapshot.failure_reason = Some(reason.to_owned());
    }

    async fn record_filesystem_error(
        &self,
        reason: &'static str,
        path: &Path,
        os_error: &std::io::Error,
    ) {
        {
            let mut snapshot = self.status.snapshot.lock().await;
            snapshot.summary.filesystem_errors =
                snapshot.summary.filesystem_errors.saturating_add(1);
        }
        tracing::error!(
            reason,
            path = %path.display(),
            os_error = %os_error,
            "Filesystem error during Music scan"
        );
    }
}

#[derive(Clone)]
pub(crate) struct ScanCoordinator {
    music_directory: MusicDirectory,
    latest_scan: Arc<Mutex<Option<ScanHandle>>>,
}

impl ScanCoordinator {
    pub(crate) fn new(music_directory: MusicDirectory) -> Self {
        Self {
            music_directory,
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
        let result = {
            let _music_dir_guard = self.music_directory.lock().await;
            discover_album_candidates(self.music_directory.path(), &scan).await
        };

        match result {
            Ok(candidates) => {
                let candidate_count = u32::try_from(candidates.len()).unwrap_or(u32::MAX);
                scan.complete(candidate_count).await;
            }
            Err(()) => scan.fail("Could not scan the Music directory.").await,
        }
    }
}

async fn discover_album_candidates(
    configured_root: &Path,
    scan: &ActiveScan,
) -> Result<Vec<AlbumCandidate>, ()> {
    let music_root = match tokio::fs::canonicalize(configured_root).await {
        Ok(root) => root,
        Err(error) => {
            scan.record_filesystem_error("resolve_music_root", configured_root, &error)
                .await;
            return Err(());
        }
    };
    Scanner { music_root, scan }.discover().await
}

struct Scanner<'a> {
    music_root: PathBuf,
    scan: &'a ActiveScan,
}

impl Scanner<'_> {
    async fn discover(&self) -> Result<Vec<AlbumCandidate>, ()> {
        let artist_entries = self
            .list_directory_entries(&self.music_root)
            .await
            .ok_or(())?;
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
                snapshot.summary.album_directories_found =
                    snapshot.summary.album_directories_found.saturating_add(1);
            }
        }

        Ok(candidates)
    }

    async fn list_directory_entries(
        &self,
        absolute_path: &Path,
    ) -> Option<Vec<tokio::fs::DirEntry>> {
        let mut directory = match tokio::fs::read_dir(absolute_path).await {
            Ok(directory) => directory,
            Err(error) => {
                self.scan
                    .record_filesystem_error("read_directory", absolute_path, &error)
                    .await;
                return None;
            }
        };
        let mut entries = Vec::new();
        loop {
            match directory.next_entry().await {
                Ok(Some(entry)) => entries.push(entry),
                Ok(None) => return Some(entries),
                Err(error) => {
                    self.scan
                        .record_filesystem_error("read_directory_entry", absolute_path, &error)
                        .await;
                    return Some(entries);
                }
            }
        }
    }

    async fn classify_entry(&self, entry: &tokio::fs::DirEntry) -> Option<ScannedEntryKind> {
        let file_type = match entry.file_type().await {
            Ok(file_type) => file_type,
            Err(error) => {
                self.scan
                    .record_filesystem_error("read_entry_type", &entry.path(), &error)
                    .await;
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

    async fn contains_audio_file(&self, album_root: &Path) -> bool {
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

    async fn record_skipped(&self, reason: &'static str, path: &Path) {
        {
            let mut snapshot = self.scan.status.snapshot.lock().await;
            snapshot.summary.skipped_directories =
                snapshot.summary.skipped_directories.saturating_add(1);
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
mod tests {
    use super::*;

    fn test_scan() -> ActiveScan {
        ActiveScan::new().0
    }

    async fn discover(root: &Path) -> Vec<AlbumCandidate> {
        discover_album_candidates(root, &test_scan()).await.unwrap()
    }

    async fn summary(scan: &ActiveScan) -> ScanSummary {
        scan.status.snapshot.lock().await.summary.clone()
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
        let candidates = discover_album_candidates(music.path(), &scan)
            .await
            .unwrap();

        assert!(candidates.is_empty());
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
        let candidates = discover_album_candidates(&root_link, &scan).await.unwrap();

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
        let candidates = discover_album_candidates(music.path(), &scan)
            .await
            .unwrap();

        assert!(candidates.is_empty());
        assert_eq!(summary(&scan).await.skipped_directories, 1);
    }

    #[tokio::test]
    async fn a_missing_root_is_a_terminal_failure() {
        let root = tempfile::tempdir().unwrap().path().join("missing");
        let scan = test_scan();

        assert!(discover_album_candidates(&root, &scan).await.is_err());
        assert_eq!(summary(&scan).await.filesystem_errors, 1);
    }

    #[tokio::test]
    async fn coordinator_retains_a_terminal_failure() {
        let root = tempfile::tempdir().unwrap().path().join("missing");
        let coordinator = ScanCoordinator::new(MusicDirectory::new(root));

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
        assert_eq!(coordinator.snapshot().await, Some(failed));
    }
}
