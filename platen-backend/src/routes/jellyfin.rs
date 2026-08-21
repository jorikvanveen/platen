use axum::{Json, extract::State};
use reqwest::StatusCode;
use serde::Serialize;
use tracing::{error, info};

use crate::{AppState, services::jellyfin::JellyfinError};

impl From<JellyfinError> for StatusCode {
    fn from(e: JellyfinError) -> Self {
        match e {
            JellyfinError::Unreachable => StatusCode::SERVICE_UNAVAILABLE,
            JellyfinError::Auth => StatusCode::UNAUTHORIZED,
            e => {
                error!("Jellyfin import failed: {e:?}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ImportFailure {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ImportSummary {
    pub total_scanned: u32,
    pub created: u32,
    pub linked: u32,
    pub skipped: u32,
    pub failed: u32,
    pub failures: Vec<ImportFailure>,
}

pub async fn import(
    State(AppState { jellyfin, .. }): State<AppState>,
) -> Result<Json<ImportSummary>, StatusCode> {
    info!("Starting Jellyfin import");

    let albums = jellyfin.list_albums().await?;

    Ok(Json(ImportSummary {
        total_scanned: albums.len() as u32,
        created: 0,
        linked: 0,
        skipped: 0,
        failed: 0,
        failures: Vec::new(),
    }))
}
