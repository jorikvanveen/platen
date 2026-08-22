use axum::{Json, extract::State};
use reqwest::StatusCode;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter, Set,
};
use tracing::{error, info, warn};

use crate::{
    AppState,
    entity::{album, artist},
    services::musicbrainz::RequestError as MbRequestError,
};

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

#[axum::debug_handler]
pub async fn import(
    State(AppState {
        musicbrainz,
        jellyfin,
        tidal,
        db,
        ..
    }): State<AppState>,
) -> Result<Json<dto::ImportSummary>, StatusCode> {
    info!("Starting Jellyfin import");

    let jellyfin_albums = jellyfin.list_albums().await.map_err(|e| {
        error!("Jellyfin list_albums failed: {e:#?}");
        StatusCode::BAD_GATEWAY
    })?;

    let mut summary = dto::ImportSummary {
        total_scanned: jellyfin_albums.len() as u32,
        created: 0,
        linked: 0,
        skipped: 0,
        failed: 0,
        failures: Vec::new(),
    };

    for jf_album in jellyfin_albums {
        let mb_release_group_id = jf_album
            .provider_ids
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("musicbrainzreleasegroup"))
            .map(|(_, v)| v.clone());
        let mb_artist_id = jf_album
            .provider_ids
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("musicbrainzartist"))
            .map(|(_, v)| v.clone());

        let Some(mb_release_group_id) = mb_release_group_id else {
            summary.skipped += 1;
            continue;
        };

        match resolve_album(
            &db,
            &musicbrainz,
            &tidal,
            &jf_album,
            mb_release_group_id,
            mb_artist_id,
        )
        .await
        {
            Ok(Outcome::Linked) => summary.linked += 1,
            Ok(Outcome::Created) => summary.created += 1,
            Ok(Outcome::Skipped) => summary.skipped += 1,
            Err(reason) => {
                summary.failed += 1;
                summary
                    .failures
                    .push(dto::ImportFailure { name: jf_album.name, reason });
            }
        }
    }

    Ok(Json(summary))
}

enum Outcome {
    Linked,
    Created,
    Skipped,
}

/// Resolve a single Jellyfin album to a Tidal-keyed `album` row.
async fn resolve_album(
    db: &sea_orm::DatabaseConnection,
    musicbrainz: &crate::services::musicbrainz::Musicbrainz,
    tidal: &std::sync::Arc<tokio::sync::Mutex<crate::services::tidal::Tidal>>,
    jf_album: &crate::services::jellyfin::JellyfinAlbum,
    mb_release_group_id: String,
    mb_artist_id: Option<String>,
) -> Result<Outcome, String> {
    // 1. Look up an existing album by its MusicBrainz ID.
    let existing = album::Entity::find()
        .filter(album::Column::MusicbrainzId.eq(mb_release_group_id.clone()))
        .one(db)
        .await
        .map_err(|e| {
            error!("Db error looking up album by MBID: {e:#?}");
            "database error".to_string()
        })?;

    if let Some(existing) = existing {
        if existing.jellyfin_id.is_some() {
            return Ok(Outcome::Skipped);
        }
        let mut active: album::ActiveModel = existing.into();
        active.jellyfin_id = ActiveValue::Set(Some(jf_album.id.clone()));
        active.update(db).await.map_err(|e| {
            error!("Db error linking existing album: {e:#?}");
            "database error".to_string()
        })?;
        return Ok(Outcome::Linked);
    }

    // 2. No row by MBID. Fetch the MB release group for title + artist.
    let mb_rg = musicbrainz
        .get_release_group(&mb_release_group_id)
        .await
        .map_err(|e| match e {
            MbRequestError::Reqwest(err) => {
                error!("MB reqwest error: {err:#?}");
                "musicbrainz request failed".to_string()
            }
            MbRequestError::MusicbrainzError(status, body) => {
                error!("MB error {status}: {body}");
                format!("musicbrainz returned {status}")
            }
        })?;

    let title = mb_rg.title.clone();
    let artist_name = mb_rg
        .artist_credit
        .as_ref()
        .and_then(|credits| credits.first())
        .map(|c| c.name.clone())
        .ok_or_else(|| "release group has no artist credit".to_string())?;

    // 3. Search Tidal by "{artist} {title}".
    let query = format!("{artist_name} {title}");
    let first_hit = {
        let mut tidal = tidal.lock().await;
        let hits = tidal.find_album(&query).await.map_err(|e| {
            error!("Tidal find_album failed: {e:#?}");
            "tidal search failed".to_string()
        })?;
        hits.into_iter().next().ok_or_else(|| {
            warn!("No Tidal hits for {query:?}");
            "no tidal matches".to_string()
        })?
    };

    // 4. Follow up with GET /albums/{id}?include=artists for the Tidal artist.
    let tidal_artists = {
        let mut tidal = tidal.lock().await;
        tidal.get_album_artists(&first_hit.id).await.map_err(|e| {
            error!("Tidal get_album_artists failed: {e:#?}");
            "tidal album fetch failed".to_string()
        })?
    };
    let tidal_artist = tidal_artists
        .into_iter()
        .next()
        .ok_or_else(|| "tidal album has no artists".to_string())?;

    // 5. Upsert the artist row keyed by Tidal artist ID.
    let existing_artist = artist::Entity::find_by_id(tidal_artist.id.clone())
        .one(db)
        .await
        .map_err(|e| {
            error!("Db error looking up artist: {e:#?}");
            "database error".to_string()
        })?;

    match existing_artist {
        Some(existing) => {
            // Backfill the MBID if the existing row lacks one and Jellyfin provided it.
            if existing.musicbrainz_id.is_none() && mb_artist_id.is_some() {
                let mut active: artist::ActiveModel = existing.into();
                active.musicbrainz_id = Set(mb_artist_id.clone());
                active.update(db).await.map_err(|e| {
                    error!("Db error updating artist MBID: {e:#?}");
                    "database error".to_string()
                })?;
            }
        }
        None => {
            artist::ActiveModel {
                id: ActiveValue::Set(tidal_artist.id.clone()),
                name: ActiveValue::Set(tidal_artist.name.clone()),
                musicbrainz_id: ActiveValue::Set(mb_artist_id.clone()),
            }
            .insert(db)
            .await
            .map_err(|e| {
                error!("Db error inserting artist: {e:#?}");
                "database error".to_string()
            })?;
        }
    }

    // 6. Insert the album row, or link it if a row with this Tidal ID already
    // exists (e.g. created earlier via the Tidal-by-ID creation route).
    let existing_by_tidal_id = album::Entity::find_by_id(first_hit.id.clone())
        .one(db)
        .await
        .map_err(|e| {
            error!("Db error looking up album by Tidal ID: {e:#?}");
            "database error".to_string()
        })?;

    if let Some(existing) = existing_by_tidal_id {
        let jellyfin_id = if existing.jellyfin_id.is_none() {
            Some(jf_album.id.clone())
        } else {
            existing.jellyfin_id.clone()
        };
        let musicbrainz_id = if existing.musicbrainz_id.is_none() {
            Some(mb_release_group_id.clone())
        } else {
            existing.musicbrainz_id.clone()
        };
        let mut active: album::ActiveModel = existing.into();
        active.jellyfin_id = Set(jellyfin_id);
        active.musicbrainz_id = Set(musicbrainz_id);
        active.update(db).await.map_err(|e| {
            error!("Db error linking album by Tidal ID: {e:#?}");
            "database error".to_string()
        })?;
        return Ok(Outcome::Linked);
    }

    album::ActiveModel {
        id: ActiveValue::Set(first_hit.id.clone()),
        artist_id: ActiveValue::Set(tidal_artist.id.clone()),
        title: ActiveValue::Set(title),
        album_type: ActiveValue::Set(first_hit.album_type.clone()),
        jellyfin_id: ActiveValue::Set(Some(jf_album.id.clone())),
        musicbrainz_id: ActiveValue::Set(Some(mb_release_group_id.clone())),
        match_method: ActiveValue::Set(Some("name_search".to_string())),
    }
    .insert(db)
    .await
    .map_err(|e| {
        error!("Db error inserting album: {e:#?}");
        "database error".to_string()
    })?;

    Ok(Outcome::Created)
}
