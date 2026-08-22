use axum::{Json, extract::State};
use reqwest::StatusCode;
use tracing::info;

use crate::AppState;

pub mod dto {
    use serde::Serialize;
    use ts_rs::TS;

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct ImportFailure {
        pub name: String,
        pub reason: String,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct ImportSummary {
        pub total_scanned: u32,
        pub created: u32,
        pub linked: u32,
        pub skipped: u32,
        pub failed: u32,
        pub failures: Vec<ImportFailure>,
    }
}

/// Jellyfin import is offline until slice 2 rewires it to the Tidal-keyed
/// schema. Returns 501 for now.
pub async fn import(State(_): State<AppState>) -> Result<Json<dto::ImportSummary>, StatusCode> {
    info!("Jellyfin import is not implemented in this slice");
    Err(StatusCode::NOT_IMPLEMENTED)
}
