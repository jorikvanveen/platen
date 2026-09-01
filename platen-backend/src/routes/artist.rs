use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::StatusCode;
use sea_orm::EntityTrait;
use tracing::{error, info};

use crate::{app::AppState, entity::artist};

pub mod dto {
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct Artist {
        pub id: String,
        pub name: String,
        pub profile_image_url: Option<String>,
    }
}

impl From<artist::Model> for dto::Artist {
    fn from(model: artist::Model) -> Self {
        dto::Artist {
            id: model.id,
            name: model.name,
            profile_image_url: model.profile_image_url,
        }
    }
}

#[axum::debug_handler]
pub async fn get(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<dto::Artist>, StatusCode> {
    let artist = artist::Entity::find_by_id(id)
        .one(&db)
        .await
        .map_err(|e| {
            error!("DB Error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(artist.into()))
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
