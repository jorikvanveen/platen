use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::StatusCode;
use sea_orm::{ActiveModelTrait, ActiveValue, EntityTrait};
use tracing::{error, info};

use crate::{AppState, entity::artist, routes::utils};

pub mod dto {
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct Artist {
        pub id: String,
        pub name: String,
        pub musicbrainz_artist_id: Option<String>,
    }
}

impl From<artist::Model> for dto::Artist {
    fn from(model: artist::Model) -> Self {
        dto::Artist {
            id: model.id,
            name: model.name,
            musicbrainz_artist_id: model.musicbrainz_artist_id,
        }
    }
}

#[axum::debug_handler]
pub async fn get(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<dto::Artist>, StatusCode> {
    match artist::Entity::find_by_id(id).one(&db).await {
        Ok(artist) => match artist {
            Some(artist) => Ok(Json(artist.into())),
            None => Err(StatusCode::NOT_FOUND),
        },
        Err(e) => {
            error!("DB Error: {:#?}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[axum::debug_handler]
pub async fn create(
    State(AppState { tidal, db, .. }): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<dto::Artist>, StatusCode> {
    info!("Creating artist {id}");
    let tidal_artist = tidal
        .get_artist(&id)
        .await
        .map_err(utils::map_tidal_error)?;

    let artist_model = artist::ActiveModel {
        id: ActiveValue::Set(tidal_artist.id),
        name: ActiveValue::Set(tidal_artist.name),
        musicbrainz_artist_id: ActiveValue::Set(None),
    };

    let result_model = artist_model.insert(&db).await.map_err(|e| {
        error!("DB error: {e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(result_model.into()))
}

pub async fn list(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<dto::Artist>>, StatusCode> {
    info!("Listing artists");
    let artists = artist::Entity::find().all(&db).await.map_err(|e| {
        error!("{:#?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(artists.into_iter().map(Into::into).collect()))
}
