use axum::{Json, extract::{Path, State}};
use reqwest::StatusCode;
use sea_orm::{ActiveModelTrait, ActiveValue, EntityTrait, ModelTrait};
use tracing::{error, info};

use crate::{AppState, entity};

pub async fn create(
    State(AppState { musicbrainz, db, .. }): State<AppState>,
    Path((artist_id, release_id)): Path<(String, String)>,
) -> Result<Json<entity::release::Model>, StatusCode> {
    info!("Creating release {release_id} on artist {artist_id}");
    let release = musicbrainz.get_release(&release_id).await.map_err(|e| match e {
        crate::musicbrainz::RequestError::Reqwest(error) => {
            error!("Reqwest: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        },
        crate::musicbrainz::RequestError::MusicbrainzError(StatusCode::NOT_FOUND, _) => StatusCode::NOT_FOUND,
        crate::musicbrainz::RequestError::MusicbrainzError(status, error) => {
            error!("Musicbrainz: {status}: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        },
    })?;

    let artist_credit = release.artist_credit.first().ok_or_else(|| {
        error!("Release does not have artist credit");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if artist_credit.artist.id != artist_id {
        error!("Tried to create release on invalid artist");
        return Err(StatusCode::BAD_REQUEST)
    }
    
    let model = entity::release::ActiveModel {
        artist_id: ActiveValue::Set(artist_credit.artist.id.clone()),
        musicbrainz_id: ActiveValue::Set(release.id),
        downloaded: ActiveValue::Set(false),
        title: ActiveValue::Set(release.title)
    }.insert(&db).await.map_err(|e| {
        error!("Db error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(model))
}

pub async fn fetch_all(
    State(AppState { db, .. }): State<AppState>,
    Path(artist_id): Path<String>
) -> Result<Json<Vec<entity::release::Model>>, StatusCode> {
    let artist = entity::artist::Entity::find_by_id(&artist_id).one(&db).await.map_err(|e| {
        error!("Db error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?.ok_or(StatusCode::NOT_FOUND)?;
    info!("Found artist: {:?}", artist);

    let releases = artist.find_related(entity::release::Entity).all(&db).await.map_err(|e| {
        error!("Db error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    
    Ok(Json(releases))
}
