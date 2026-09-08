use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use super::model::AlbumCandidate;
use crate::services::music_directory::STAGING_DIRECTORY;

#[derive(Default)]
pub(super) struct DiscoveryReport {
    pub(super) resolved_root: Option<PathBuf>,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DiscoveryProgress {
    pub(super) album_directories_found: usize,
    pub(super) skipped_directories: usize,
    pub(super) filesystem_errors: usize,
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

pub(super) async fn discover_album_candidates(
    configured_root: &Path,
    mut publish: impl FnMut(DiscoveryProgress) + Send,
) -> DiscoveryReport {
    let music_root = match tokio::fs::canonicalize(configured_root).await {
        Ok(root) => root,
        Err(error) => {
            let diagnostic = FilesystemDiagnostic {
                reason: "resolve_music_root",
                path: configured_root.to_owned(),
                os_error: error,
            };
            diagnostic.log();
            publish(DiscoveryProgress {
                filesystem_errors: 1,
                ..Default::default()
            });
            return DiscoveryReport {
                diagnostics: vec![diagnostic],
                root_failed: true,
                ..Default::default()
            };
        }
    };
    let mut scanner = Scanner {
        music_root,
        publish,
        progress: DiscoveryProgress::default(),
        diagnostics: Vec::new(),
    };
    let result = scanner.discover().await;
    DiscoveryReport {
        resolved_root: Some(scanner.music_root),
        root_failed: result.is_err(),
        candidates: result.unwrap_or_default(),
        diagnostics: scanner.diagnostics,
        skipped_directories: scanner.progress.skipped_directories,
    }
}

struct Scanner<F> {
    music_root: PathBuf,
    publish: F,
    progress: DiscoveryProgress,
    diagnostics: Vec<FilesystemDiagnostic>,
}

impl<F: FnMut(DiscoveryProgress) + Send> Scanner<F> {
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
                self.record_skipped("unreadable_artist_directory", &artist_entry.path());
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
                    self.record_skipped("malformed_album_directory", &album_entry.path());
                    continue;
                };
                if !self.contains_audio_file(&album_entry.path()).await {
                    self.record_skipped("no_supported_audio", &album_entry.path());
                    continue;
                }
                candidates.push(AlbumCandidate {
                    primary_artist: artist_name.clone(),
                    title,
                    release_year,
                    relative_path: format!("{artist_name}/{album_name}"),
                });
                self.progress.album_directories_found = candidates.len();
                (self.publish)(self.progress.clone());
            }
        }
        candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(candidates)
    }

    async fn list_directory_entries(&mut self, path: &Path) -> Option<Vec<tokio::fs::DirEntry>> {
        let mut directory = match tokio::fs::read_dir(path).await {
            Ok(directory) => directory,
            Err(error) => {
                self.record_filesystem_error("read_directory", path, error);
                return None;
            }
        };
        let mut entries = Vec::new();
        loop {
            match directory.next_entry().await {
                Ok(Some(entry)) => entries.push(entry),
                Ok(None) => return Some(entries),
                Err(error) => {
                    self.record_filesystem_error("read_directory_entry", path, error);
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
            self.record_skipped("symbolic_link", &entry.path());
            return None;
        }
        if file_type.is_dir() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                self.record_skipped("non_utf8_directory", &entry.path());
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
                    Some(ScannedEntryKind::Directory(_)) => pending.push_back(entry.path()),
                    Some(ScannedEntryKind::File) if is_audio_file(&entry.path()) => {
                        found_audio = true
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
        self.progress.filesystem_errors = self.diagnostics.len();
        (self.publish)(self.progress.clone());
    }

    fn record_skipped(&mut self, reason: &'static str, path: &Path) {
        self.progress.skipped_directories += 1;
        (self.publish)(self.progress.clone());
        tracing::warn!(reason, path = %path.display(), "Skipped directory during Music scan");
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
#[path = "../../tests/services/import/discovery.rs"]
mod tests;
