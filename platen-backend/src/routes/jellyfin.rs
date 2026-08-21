use axum::{Json, extract::State};
use reqwest::StatusCode;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::Serialize;
use tracing::{error, info, warn};

use crate::{
    AppState,
    entity::release_group,
    services::jellyfin::{JellyfinAlbum, JellyfinError},
};

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
    State(AppState { jellyfin, db, .. }): State<AppState>,
) -> Result<Json<ImportSummary>, StatusCode> {
    info!("Starting Jellyfin import");

    let albums = jellyfin.list_albums().await?;

    let mut created = 0u32;
    let mut linked = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;
    let mut failures = Vec::new();

    for album in &albums {
        match handle_album(album, &db).await {
            Ok(Outcome::Created) => created += 1,
            Ok(Outcome::Linked) => linked += 1,
            Ok(Outcome::Skipped) => skipped += 1,
            Err(reason) => {
                warn!("Jellyfin import failure for {}: {reason}", album.name);
                failed += 1;
                failures.push(ImportFailure {
                    name: album.name.clone(),
                    reason,
                });
            }
        }
    }

    Ok(Json(ImportSummary {
        total_scanned: albums.len() as u32,
        created,
        linked,
        skipped,
        failed,
        failures,
    }))
}

enum Outcome {
    Created,
    Linked,
    Skipped,
}

async fn handle_album(album: &JellyfinAlbum, db: &sea_orm::DatabaseConnection) -> Result<Outcome, String> {
    let mb_id = find_musicbrainz_release_group(&album.provider_ids);

    let mb_id = match mb_id {
        Some(id) => id,
        None => {
            info!("Skipping {}: no MusicBrainz release group id", album.name);
            return Ok(Outcome::Skipped);
        }
    };

    let existing = release_group::Entity::find_by_id(mb_id)
        .one(db)
        .await
        .map_err(|e| e.to_string())?;

    let Some(existing) = existing else {
        info!("Skipping {}: not in platen, no create yet", album.name);
        return Ok(Outcome::Skipped);
    };

    if existing.jellyfin_id.is_some() {
        info!("Skipping {}: already linked", album.name);
        return Ok(Outcome::Skipped);
    }

    let mut model: release_group::ActiveModel = existing.into();
    model.jellyfin_id = Set(Some(album.id.clone()));
    model.downloaded = Set(true);
    model
        .update(db)
        .await
        .map_err(|e| e.to_string())?;

    info!("Linked {} to Jellyfin id {}", album.name, album.id);
    Ok(Outcome::Linked)
}

fn find_musicbrainz_release_group(provider_ids: &std::collections::HashMap<String, String>) -> Option<&str> {
    provider_ids
        .iter()
        .find_map(|(key, value)| {
            if key.eq_ignore_ascii_case("musicbrainzreleasegroup") {
                Some(value.as_str())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_musicbrainz_release_group_case_insensitively() {
        let mut ids = std::collections::HashMap::new();
        ids.insert("MusicBrainzReleaseGroup".to_string(), "mb-rg".to_string());
        assert_eq!(find_musicbrainz_release_group(&ids), Some("mb-rg"));

        let mut ids = std::collections::HashMap::new();
        ids.insert("musicbrainzreleasegroup".to_string(), "lower".to_string());
        assert_eq!(find_musicbrainz_release_group(&ids), Some("lower"));

        let ids = std::collections::HashMap::new();
        assert_eq!(find_musicbrainz_release_group(&ids), None);
    }
}
