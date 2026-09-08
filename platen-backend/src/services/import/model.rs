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

impl Default for ScanSnapshot {
    fn default() -> Self {
        Self {
            phase: ScanPhase::Scanning,
            summary: ScanSummary::default(),
            failure_reason: None,
        }
    }
}

impl ScanSnapshot {
    pub(super) fn fail(&mut self, reason: &str) {
        self.phase = ScanPhase::Failed;
        self.summary.failures += 1;
        self.failure_reason = Some(reason.to_owned());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AlbumCandidate {
    pub(super) primary_artist: String,
    pub(super) title: String,
    pub(super) release_year: Option<i32>,
    pub(super) relative_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CatalogAlbum {
    pub(super) id: String,
    pub(super) primary_artist: Option<String>,
    pub(super) title: String,
    pub(super) release_year: i32,
    pub(super) relative_path: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct LocationUpdate {
    pub(super) album_id: String,
    pub(super) previous_path: Option<String>,
    pub(super) relative_path: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum LocationOutcome {
    Attached,
    Changed,
    Cleared,
    Unchanged,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ImportOutcome {
    Imported,
    Location(LocationOutcome),
    Duplicate { stored_path: Option<String> },
}
