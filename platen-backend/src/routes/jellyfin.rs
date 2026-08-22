use axum::{Json, extract::State};
use reqwest::StatusCode;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use tracing::{error, info, warn};

use crate::{
    AppState,
    entity::{artist, release_group},
    services::{
        jellyfin::{JellyfinAlbum, JellyfinError},
        musicbrainz::{Musicbrainz, RequestError},
    },
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

pub async fn import(
    State(AppState { musicbrainz, jellyfin, db, .. }): State<AppState>,
) -> Result<Json<dto::ImportSummary>, StatusCode> {
    info!("Starting Jellyfin import");

    let albums = jellyfin.list_albums().await?;

    let mut created = 0u32;
    let mut linked = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;
    let mut failures = Vec::new();

    for album in &albums {
        match handle_album(album, &db, &musicbrainz).await {
            Ok(Outcome::Created) => created += 1,
            Ok(Outcome::Linked) => linked += 1,
            Ok(Outcome::Skipped) => skipped += 1,
            Err(reason) => {
                warn!("Jellyfin import failure for {}: {reason}", album.name);
                failed += 1;
                failures.push(dto::ImportFailure {
                    name: album.name.clone(),
                    reason,
                });
            }
        }
    }

    Ok(Json(dto::ImportSummary {
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

async fn handle_album(
    album: &JellyfinAlbum,
    db: &sea_orm::DatabaseConnection,
    musicbrainz: &Musicbrainz,
) -> Result<Outcome, String> {
    let mb_id = find_musicbrainz_release_group(&album.provider_ids);

    let mb_id = match mb_id {
        Some(id) => id.to_string(),
        None => {
            info!("Skipping {}: no MusicBrainz release group id", album.name);
            return Ok(Outcome::Skipped);
        }
    };

    let existing = release_group::Entity::find_by_id(&mb_id)
        .one(db)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(existing) = existing {
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
        return Ok(Outcome::Linked);
    }

    let group = musicbrainz
        .get_release_group(&mb_id)
        .await
        .map_err(|e| match e {
            RequestError::MusicbrainzError(StatusCode::NOT_FOUND, _) => "MusicBrainz release group not found".to_string(),
            e => e.to_string(),
        })?;

    let credit = group
        .artist_credit
        .as_ref()
        .and_then(|credits| credits.first())
        .ok_or_else(|| "missing artist_credit".to_string())?;

    let artist_id = credit.artist.id.clone();
    let artist_name = credit.artist.name.clone();

    let exising_artist = artist::Entity::find_by_id(&artist_id)
        .one(db)
        .await
        .map_err(|e| e.to_string())?;

    if exising_artist.is_none() {
        let artist = artist::ActiveModel {
            musicbrainz_id: Set(artist_id.clone()),
            name: Set(artist_name),
        };
        artist
            .insert(db)
            .await
            .map_err(|e| e.to_string())?;
        info!("Created artist {artist_id}");
    }

    let model = release_group::ActiveModel {
        musicbrainz_id: Set(mb_id),
        title: Set(group.title),
        artist_id: Set(artist_id),
        r#type: Set(group.primary_type),
        downloaded: Set(true),
        jellyfin_id: Set(Some(album.id.clone())),
    };
    model
        .insert(db)
        .await
        .map_err(|e| e.to_string())?;

    info!("Created release group {} for Jellyfin id {}", album.name, album.id);
    Ok(Outcome::Created)
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
