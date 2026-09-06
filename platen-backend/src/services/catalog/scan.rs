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
#[path = "../../tests/services/catalog/scan.rs"]
mod tests;
