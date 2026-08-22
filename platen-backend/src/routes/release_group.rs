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
    entity::{self, release_group},
    services::downloaders::Downloader,
};

pub mod dto {
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct ReleaseGroup {
        pub musicbrainz_id: String,
        pub title: String,
        pub artist_id: String,
        #[ts(rename = "type")]
        pub r#type: String,
        pub downloaded: bool,
        pub jellyfin_id: Option<String>,
    }
}

impl From<release_group::Model> for dto::ReleaseGroup {
    fn from(model: release_group::Model) -> Self {
        dto::ReleaseGroup {
            musicbrainz_id: model.musicbrainz_id,
            title: model.title,
            artist_id: model.artist_id,
            r#type: model.r#type,
            downloaded: model.downloaded,
            jellyfin_id: model.jellyfin_id,
        }
    }
}

pub async fn create(
    State(AppState {
        musicbrainz, db, ..
    }): State<AppState>,
    Path((artist_id, release_group_id)): Path<(String, String)>,
) -> Result<Json<dto::ReleaseGroup>, StatusCode> {
    info!("Creating release group {release_group_id} on artist {artist_id}");
    let group = musicbrainz
        .get_release_group(&release_group_id)
        .await
        .map_err(|e| match e {
            crate::services::musicbrainz::RequestError::Reqwest(error) => {
                error!("Reqwest: {error:#?}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            crate::services::musicbrainz::RequestError::MusicbrainzError(
                StatusCode::NOT_FOUND,
                _,
            ) => StatusCode::NOT_FOUND,
            crate::services::musicbrainz::RequestError::MusicbrainzError(status, error) => {
                error!("Musicbrainz: {status}: {error:#?}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;

    let model = entity::release_group::ActiveModel {
        artist_id: ActiveValue::Set(artist_id.clone()),
        musicbrainz_id: ActiveValue::Set(group.id),
        downloaded: ActiveValue::Set(false),
        title: ActiveValue::Set(group.title),
        r#type: ActiveValue::Set(group.primary_type),
        jellyfin_id: ActiveValue::Set(None),
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
) -> Result<Json<Vec<dto::ReleaseGroup>>, StatusCode> {
    let artist = entity::artist::Entity::find_by_id(&artist_id)
        .one(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    info!("Found artist: {:?}", artist);

    let release_groups = artist
        .find_related(entity::release_group::Entity)
        .all(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(release_groups.into_iter().map(Into::into).collect()))
}

pub async fn download(
    State(AppState {
        antra, db, config, ..
    }): State<AppState>,
    Path((artist_id, release_group_id)): Path<(String, String)>,
) -> Result<(), StatusCode> {
    let artist = entity::artist::Entity::find_by_id(&artist_id)
        .one(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let release_group = artist
        .find_related(entity::release_group::Entity)
        .filter(entity::release_group::Column::MusicbrainzId.eq(release_group_id.clone()))
        .one(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    antra
        .download_release_group(
            &artist.name,
            &release_group.title,
            &PathBuf::from(config.music_dir),
        )
        .await
        .map_err(|e| {
            error!("Download failed: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let release_group = entity::release_group::ActiveModel {
        musicbrainz_id: ActiveValue::Unchanged(release_group_id),
        downloaded: ActiveValue::Set(true),
        ..Default::default()
    };

    release_group.save(&db).await.map_err(|e| {
        error!("Failed to mark release group as downloaded: {e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(())
}
