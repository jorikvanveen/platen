use axum::extract::*;
use reqwest::StatusCode;
use sea_orm::{ActiveModelTrait, ActiveValue, EntityTrait};
use tracing::{error, info};

use crate::{AppState, entity::artist, musicbrainz::RequestError};

#[axum::debug_handler]
pub async fn get(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<String>
) -> Result<Json<artist::Model>, StatusCode> {
    match artist::Entity::find_by_id(id).one(&db).await {
        Ok(artist) => match artist {
            Some(artist) => Ok(Json(artist)),
            None => Err(StatusCode::NOT_FOUND)
        },
        Err(e) => {
            error!("DB Error: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

//#[instrument]
#[axum::debug_handler]
pub async fn create(
    State(AppState { musicbrainz, db, .. }): State<AppState>,
    Path(id): Path<String>
) -> Result<Json<artist::Model>, StatusCode> {
    info!("Creating artist {id}");
    let artist = musicbrainz.get_artist(&id).await.map_err(|e| match e {
        RequestError::Reqwest(error) => {
            error!("Reqwest: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        },
        RequestError::MusicbrainzError(StatusCode::NOT_FOUND, _) => StatusCode::NOT_FOUND,
        RequestError::MusicbrainzError(status, error) => {
            error!("Musicbrainz: {status}: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        },
    })?;

    let artist_model = artist::ActiveModel {
        musicbrainz_id: ActiveValue::Set(artist.id),
        name: ActiveValue::Set(artist.name)
    };
    
    let result_model = artist_model.insert(&db).await.map_err(|e| {
        error!("DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    
    Ok(Json(result_model))
}

pub async fn list(
    State(AppState { db, .. }): State<AppState>
) -> Result<Json<Vec<artist::Model>>, StatusCode> {
    info!("Listing artists");
    let artists = artist::Entity::find().all(&db).await.map_err(|e| {
        error!("{}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(artists))
}
