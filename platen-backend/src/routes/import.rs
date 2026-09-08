use crate::app::AppState;
use axum::{Json, extract::State, response::IntoResponse};
use reqwest::StatusCode;

pub mod dto {
    use crate::services::import::{ScanPhase, ScanSnapshot, ScanSummary};
    use serde::Serialize;
    use ts_rs::TS;

    #[derive(Debug, PartialEq, Eq, Serialize, TS)]
    #[serde(rename_all = "snake_case")]
    #[ts(export, rename_all = "snake_case")]
    pub enum CatalogScanPhase {
        Scanning,
        Matching,
        Completed,
        Failed,
    }

    impl From<ScanPhase> for CatalogScanPhase {
        fn from(phase: ScanPhase) -> Self {
            match phase {
                ScanPhase::Scanning => Self::Scanning,
                ScanPhase::Matching => Self::Matching,
                ScanPhase::Completed => Self::Completed,
                ScanPhase::Failed => Self::Failed,
            }
        }
    }

    #[derive(Debug, PartialEq, Eq, Serialize, TS)]
    #[ts(export)]
    pub struct CatalogScanSummary {
        pub album_directories_found: usize,
        pub candidates_processed: usize,
        pub candidates_total: usize,
        pub albums_imported: usize,
        pub locations_attached: usize,
        pub locations_changed: usize,
        pub unchanged_locations: usize,
        pub locations_cleared: usize,
        pub unmatched_candidates: usize,
        pub ambiguous_matches: usize,
        pub duplicate_locations: usize,
        pub skipped_directories: usize,
        pub failures: usize,
        pub filesystem_errors: usize,
    }

    impl From<ScanSummary> for CatalogScanSummary {
        fn from(summary: ScanSummary) -> Self {
            Self {
                album_directories_found: summary.album_directories_found,
                candidates_processed: summary.candidates_processed,
                candidates_total: summary.candidates_total,
                albums_imported: summary.albums_imported,
                locations_attached: summary.locations_attached,
                locations_changed: summary.locations_changed,
                unchanged_locations: summary.unchanged_locations,
                locations_cleared: summary.locations_cleared,
                unmatched_candidates: summary.unmatched_candidates,
                ambiguous_matches: summary.ambiguous_matches,
                duplicate_locations: summary.duplicate_locations,
                skipped_directories: summary.skipped_directories,
                failures: summary.failures,
                filesystem_errors: summary.filesystem_errors,
            }
        }
    }

    #[derive(Debug, PartialEq, Eq, Serialize, TS)]
    #[ts(export)]
    pub struct CatalogScan {
        pub phase: CatalogScanPhase,
        pub summary: CatalogScanSummary,
        pub failure_reason: Option<String>,
    }

    impl From<ScanSnapshot> for CatalogScan {
        fn from(scan: ScanSnapshot) -> Self {
            Self {
                phase: scan.phase.into(),
                summary: scan.summary.into(),
                failure_reason: scan.failure_reason,
            }
        }
    }
}

pub async fn start_scan(State(AppState { scan, .. }): State<AppState>) -> impl IntoResponse {
    match scan.start().await {
        Ok(status) => (StatusCode::ACCEPTED, Json(dto::CatalogScan::from(status))),
        Err(active) => (StatusCode::CONFLICT, Json(dto::CatalogScan::from(active))),
    }
}

pub async fn scan_status(
    State(AppState { scan, .. }): State<AppState>,
) -> Json<Option<dto::CatalogScan>> {
    Json(scan.snapshot().await.map(Into::into))
}
