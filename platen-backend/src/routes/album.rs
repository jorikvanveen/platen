use std::path::PathBuf;

use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::StatusCode;
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, ModelTrait, QueryFilter};
use tracing::{error, info};

use crate::{
    AppState,
    entity::{self, album},
    services::downloaders::Downloader,
};

pub mod dto {
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct Album {
        pub id: String,
        pub artist_id: String,
        pub title: String,
        pub album_type: Option<String>,
        pub jellyfin_id: Option<String>,
        pub musicbrainz_id: Option<String>,
        pub match_method: Option<String>,
    }
}

impl From<album::Model> for dto::Album {
    fn from(model: album::Model) -> Self {
        dto::Album {
            id: model.id,
            artist_id: model.artist_id,
            title: model.title,
            album_type: model.album_type,
            jellyfin_id: model.jellyfin_id,
            musicbrainz_id: model.musicbrainz_id,
            match_method: model.match_method
        }
    }
}

#[axum::debug_handler]
pub async fn create(
    State(AppState { tidal, db, .. }): State<AppState>,
    Path((artist_id, album_id)): Path<(String, String)>,
) -> Result<Json<dto::Album>, StatusCode> {
    info!("Creating album {album_id} on artist {artist_id}");
    let tidal_album = {
        let mut tidal = tidal.lock().await;
        tidal.get_album(&album_id).await.map_err(crate::routes::utils::map_tidal_error)?
    };

    let model = album::ActiveModel {
        id: ActiveValue::Set(tidal_album.id),
        artist_id: ActiveValue::Set(artist_id),
        title: ActiveValue::Set(tidal_album.title),
        album_type: ActiveValue::Set(tidal_album.album_type),
        jellyfin_id: ActiveValue::Set(None),
        musicbrainz_id: ActiveValue::Set(None),
        match_method: ActiveValue::Set(Some("tidal_id".into())),
    }
    .insert(&db)
    .await
    .map_err(|e| {
        error!("Db error: {e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(model.into()))
}

pub async fn fetch_all(
    State(AppState { db, .. }): State<AppState>,
    Path(artist_id): Path<String>,
) -> Result<Json<Vec<dto::Album>>, StatusCode> {
    let artist = entity::artist::Entity::find_by_id(&artist_id)
        .one(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    info!("Found artist: {:?}", artist);

    let albums = artist
        .find_related(album::Entity)
        .all(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(albums.into_iter().map(Into::into).collect()))
}

#[axum::debug_handler]
pub async fn download(
    State(AppState {
        antra, db, config, ..
    }): State<AppState>,
    Path((artist_id, album_id)): Path<(String, String)>,
) -> Result<(), StatusCode> {
    let artist = entity::artist::Entity::find_by_id(&artist_id)
        .one(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let album = artist
        .find_related(album::Entity)
        .filter(album::Column::Id.eq(album_id.clone()))
        .one(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    antra
        .download_album(&album, &PathBuf::from(config.music_dir))
        .await
        .map_err(|e| {
            error!("Download failed: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(())
}
