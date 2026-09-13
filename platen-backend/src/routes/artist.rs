use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::StatusCode;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, QueryTrait, sea_query::Expr};
use tracing::{error, info};

use crate::{
    app::AppState,
    entity::{album_artist, artist},
};

pub mod dto {
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct Artist {
        pub id: String,
        pub name: String,
        pub profile_image_url: Option<String>,
        pub monitored: bool,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[serde(deny_unknown_fields)]
    #[ts(export)]
    pub struct ArtistMonitoringUpdate {
        pub monitored: bool,
    }
}

impl From<artist::Model> for dto::Artist {
    fn from(model: artist::Model) -> Self {
        dto::Artist {
            id: model.id,
            name: model.name,
            profile_image_url: model.profile_image_url,
            monitored: model.monitored,
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

pub async fn update_monitoring(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<String>,
    Json(update): Json<dto::ArtistMonitoringUpdate>,
) -> Result<Json<dto::Artist>, StatusCode> {
    let artist = artist::Entity::update_many()
        .col_expr(artist::Column::Monitored, Expr::value(update.monitored))
        .filter(artist::Column::Id.eq(id))
        .exec_with_returning(&db)
        .await
        .map_err(|error| {
            error!(%error, "Could not update Artist monitoring preference");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .pop()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(artist.into()))
}

pub async fn delete(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    // Keep the credit check in the DELETE so a newly added Album cannot lose its Artist.
    let removal = artist::Entity::delete_many()
        .filter(artist::Column::Id.eq(&id))
        .filter(
            artist::Column::Id.not_in_subquery(
                album_artist::Entity::find()
                    .select_only()
                    .column(album_artist::Column::ArtistId)
                    .into_query(),
            ),
        )
        .exec(&db)
        .await
        .map_err(|error| {
            error!(%error, artist_id = %id, "Could not remove empty Artist");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if removal.rows_affected > 0 {
        return Ok(StatusCode::NO_CONTENT);
    }

    let artist = artist::Entity::find_by_id(&id)
        .one(&db)
        .await
        .map_err(|error| {
            error!(%error, artist_id = %id, "Could not load Artist after rejected removal");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Err(if artist.is_some() {
        StatusCode::CONFLICT
    } else {
        StatusCode::NOT_FOUND
    })
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
