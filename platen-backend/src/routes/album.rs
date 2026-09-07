use std::path::{self, PathBuf};

use chrono::{Datelike, NaiveDate};
use futures_util::{StreamExt, TryStreamExt, stream};
use tokio::fs;

use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::StatusCode;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
    TransactionTrait,
};
use tracing::{error, info};

use crate::{
    app::AppState,
    entity::{self, album, album_artist, artist},
    routes::{artist::dto::Artist, download::dto::DownloadJob},
    services::{
        self, album_files::remove_album_directory, catalog, downloaders::Downloader,
        filesystem::album_location,
    },
};

// Reserved child of the Music directory where downloads stage before
// publication, so staging never becomes a catalog location.
pub(crate) const STAGING_DIRECTORY: &str = ".platen-staging";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseDate {
    pub year: i32,
    pub month: Option<i32>,
    pub day: Option<i32>,
}

pub mod dto {
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    use super::Artist;

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct Album {
        pub id: String,
        pub artists: Vec<Artist>,
        pub title: String,
        pub cover_url: Option<String>,
        pub album_type: Option<String>,
        pub release_year: i32,
        pub release_month: Option<i32>,
        pub release_day: Option<i32>,
        pub relative_path: Option<String>,
        pub explicit: Option<bool>,
        pub media_tags: Option<Vec<String>>,
        pub available_quality: Option<String>,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct ReleaseDateRefreshSummary {
        pub updated: u32,
        pub skipped: u32,
    }

    #[derive(Debug, Default, Deserialize, Serialize, TS)]
    #[ts(export)]
    pub struct AlbumDeletionRequest {
        #[serde(default)]
        pub delete_files: bool,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct AlbumDeletionPreview {
        pub absolute_path: Option<String>,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct AlbumDeletionResult {
        pub removed_artist_ids: Vec<String>,
    }
}

fn album_dto(model: album::Model, artists: Vec<Artist>) -> dto::Album {
    let media_tags = catalog::parse_media_tags(&model);
    dto::Album {
        available_quality: services::tidal::available_quality(media_tags.as_deref())
            .map(str::to_owned),
        explicit: model.explicit,
        media_tags,
        id: model.id,
        artists,
        title: model.title,
        cover_url: model.cover_url,
        album_type: model.album_type,
        release_year: model.release_year,
        release_month: model.release_month,
        release_day: model.release_day,
        relative_path: model.relative_path,
    }
}

async fn credited_artists(
    db: &impl ConnectionTrait,
    album_id: &str,
) -> Result<Vec<Artist>, sea_orm::DbErr> {
    // A plain join only selects the from-entity's columns, so the artist
    // columns must come through find_also_related, not into_model.
    let rows = album_artist::Entity::find()
        .filter(album_artist::Column::AlbumId.eq(album_id))
        .find_also_related(artist::Entity)
        .order_by_asc(album_artist::Column::Position)
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(_, artist)| artist)
        .map(Into::into)
        .collect())
}

#[axum::debug_handler]
pub async fn create(
    State(AppState { tidal, db, .. }): State<AppState>,
    Path(album_id): Path<String>,
) -> Result<Json<dto::Album>, StatusCode> {
    create_with(&db, &tidal, &album_id).await
}

pub(crate) async fn create_with(
    db: &sea_orm::DatabaseConnection,
    tidal: &dyn services::tidal::TidalCatalog,
    album_id: &str,
) -> Result<Json<dto::Album>, StatusCode> {
    info!("Creating album {album_id}");
    if let Some(existing) = album::Entity::find_by_id(album_id)
        .one(db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    {
        let artists = credited_artists(db, &existing.id).await.map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        return Ok(Json(album_dto(existing, artists)));
    }

    let prepared = catalog::prepare_album(tidal, album_id)
        .await
        .map_err(|error| match error {
            catalog::PrepareAlbumError::Tidal(error) => {
                crate::routes::utils::map_tidal_error(error)
            }
            catalog::PrepareAlbumError::InvalidReleaseDate
            | catalog::PrepareAlbumError::InvalidCredits => StatusCode::UNPROCESSABLE_ENTITY,
            catalog::PrepareAlbumError::AlbumIdMismatch => crate::routes::utils::map_tidal_error(
                services::tidal::TidalError::UnexpectedResponse,
            ),
        })?;
    let model = catalog::persist_album(db, prepared, None)
        .await
        .map_err(|error| {
            error!("Db error creating album transaction: {error:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .model;

    let artists = credited_artists(db, &model.id).await.map_err(|e| {
        error!("Db error loading album credits: {e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(album_dto(model, artists)))
}

/// Legacy artist-scoped creation path, kept so existing clients keep working.
/// The album is still created from its Tidal ID with all credited artists;
/// `artist_id` only guards that the album's primary artist matches the path.
#[axum::debug_handler]
pub async fn create_artist_scoped(
    state: State<AppState>,
    Path((artist_id, album_id)): Path<(String, String)>,
) -> Result<Json<dto::Album>, StatusCode> {
    let album = create(state, Path(album_id)).await?;
    let primary = album.0.artists.first().ok_or(StatusCode::NOT_FOUND)?;
    if primary.id != artist_id {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(album)
}

pub(crate) fn parse_release_date(value: &str) -> Result<ReleaseDate, &'static str> {
    let parts: Vec<_> = value.split('-').collect();
    let year_value = parts.first().ok_or("missing year")?;
    if year_value.len() != 4 || !year_value.chars().all(|c| c.is_ascii_digit()) {
        return Err("invalid year");
    }
    let year = year_value.parse::<i32>().map_err(|_| "invalid year")?;
    if !(1..=9999).contains(&year) {
        return Err("year out of range");
    }

    match parts.as_slice() {
        [_] => Ok(ReleaseDate {
            year,
            month: None,
            day: None,
        }),
        [_, month] if month.len() == 2 => {
            let month = month.parse::<u32>().map_err(|_| "invalid month")?;
            if !(1..=12).contains(&month) {
                return Err("month out of range");
            }
            Ok(ReleaseDate {
                year,
                month: Some(month as i32),
                day: None,
            })
        }
        [_, month, day] if month.len() == 2 && day.len() == 2 => {
            let month = month.parse::<u32>().map_err(|_| "invalid month")?;
            let day = day.parse::<u32>().map_err(|_| "invalid day")?;
            NaiveDate::from_ymd_opt(year, month, day).ok_or("invalid date")?;
            Ok(ReleaseDate {
                year,
                month: Some(month as i32),
                day: Some(day as i32),
            })
        }
        _ => Err("invalid date format"),
    }
}

#[cfg(test)]
#[path = "../tests/routes/album.rs"]
mod tests;

pub async fn refresh_release_dates(
    State(AppState { tidal, db, .. }): State<AppState>,
) -> Result<Json<dto::ReleaseDateRefreshSummary>, StatusCode> {
    let albums = album::Entity::find()
        .filter(album::Column::ReleaseYear.eq(0))
        .all(&db)
        .await
        .map_err(|e| {
            error!("Db error loading albums without release dates: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut summary = dto::ReleaseDateRefreshSummary {
        updated: 0,
        skipped: 0,
    };
    for album in albums {
        let tidal_album = match tidal.get_album(&album.id).await {
            Ok(album) => album,
            Err(e) => {
                error!(
                    "Could not fetch release date for album {}: {e:#?}",
                    album.id
                );
                summary.skipped += 1;
                continue;
            }
        };
        let Some(date) = tidal_album.release_date.as_deref() else {
            error!("Tidal returned no release date for album {}", album.id);
            summary.skipped += 1;
            continue;
        };
        let release_date = match parse_release_date(date) {
            Ok(date) => date,
            Err(error_message) => {
                error!(
                    "Invalid Tidal release date for album {}: {error_message}",
                    album.id
                );
                summary.skipped += 1;
                continue;
            }
        };

        let mut active: album::ActiveModel = album.into();
        active.release_year = Set(release_date.year);
        active.release_month = Set(release_date.month);
        active.release_day = Set(release_date.day);
        if let Err(e) = active.update(&db).await {
            error!("Could not save release date for album: {e:#?}");
            summary.skipped += 1;
        } else {
            summary.updated += 1;
        }
    }

    Ok(Json(summary))
}

pub async fn fetch_all_artist_albums(
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

    let rows = album_artist::Entity::find()
        .filter(album_artist::Column::ArtistId.eq(&artist_id))
        .find_also_related(album::Entity)
        .order_by_asc(album_artist::Column::AlbumId)
        .all(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let result = stream::iter(rows.into_iter().filter_map(|(_, album_model)| {
        album_model.map(|album_model| {
            let db = &db;
            async move {
                let artists = credited_artists(db, &album_model.id).await?;
                Ok::<_, sea_orm::DbErr>(album_dto(album_model, artists))
            }
        })
    }))
    .buffered(20)
    .try_collect::<Vec<_>>()
    .await
    .map_err(|e| {
        error!("Db error: {e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(result))
}

#[axum::debug_handler]
pub async fn download(
    State(AppState { db, queue, .. }): State<AppState>,
    Path(album_id): Path<String>,
) -> Result<(StatusCode, Json<DownloadJob>), StatusCode> {
    let album = album::Entity::find_by_id(&album_id)
        .one(&db)
        .await
        .map_err(|error| {
            error!("Could not load album before enqueueing download: {error:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    if album.relative_path.is_some() {
        return Err(StatusCode::CONFLICT);
    }

    let job = queue.enqueue(album_id).await.map_err(|error| match error {
        services::download_queue::QueueError::Full => StatusCode::TOO_MANY_REQUESTS,
        services::download_queue::QueueError::WorkerStopped => {
            error!("Could not enqueue download job: {error:#?}");
            StatusCode::SERVICE_UNAVAILABLE
        }
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(DownloadJob::from_record(job, Some(&album))),
    ))
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DownloadError {
    #[error("could not load album metadata")]
    Catalog,
    #[error("album is already downloaded")]
    AlreadyDownloaded,
    #[error("album destination already exists")]
    DestinationExists,
    #[error("album transfer failed")]
    Transfer,
    #[error("could not save completed download")]
    Save,
}

impl DownloadError {
    pub(crate) fn client_message(&self) -> &'static str {
        match self {
            Self::Catalog => "Could not load album metadata.",
            Self::AlreadyDownloaded => "Album is already downloaded.",
            Self::DestinationExists => "Album destination already exists.",
            Self::Transfer => "Album download failed.",
            Self::Save => "Could not save the completed download.",
        }
    }
}

pub(crate) async fn download_with(
    db: &sea_orm::DatabaseConnection,
    music_dir: &str,
    downloader: &dyn Downloader,
    album_id: &str,
) -> Result<(), DownloadError> {
    let album = album::Entity::find_by_id(album_id)
        .one(db)
        .await
        .map_err(|e| {
            error!("Db error loading Album for download: {e:#?}");
            DownloadError::Catalog
        })?
        .ok_or_else(|| {
            error!("Album {album_id} no longer exists when its Download job started");
            DownloadError::Catalog
        })?;

    if album.relative_path.is_some() {
        return Err(DownloadError::AlreadyDownloaded);
    }

    let (_, primary) = album_artist::Entity::find()
        .filter(album_artist::Column::AlbumId.eq(&album.id))
        .filter(album_artist::Column::Position.eq(0))
        .find_also_related(artist::Entity)
        .one(db)
        .await
        .map_err(|e| {
            error!("Db error loading Primary artist: {e:#?}");
            DownloadError::Catalog
        })?
        .ok_or_else(|| {
            error!("Album {album_id} has no Primary artist relation");
            DownloadError::Catalog
        })?;
    let primary = primary.ok_or_else(|| {
        error!("Album {album_id} references a missing Primary artist");
        DownloadError::Catalog
    })?;

    // The library layout is derived from the catalog for every release type,
    // never from the archive's own structure (ADR 0003). A release year of 0
    // means the date has not been refreshed from Tidal yet; the current year
    // keeps the directory from being named "(0)" in the meantime.
    let music_dir = PathBuf::from(music_dir);
    let release_year = if album.release_year == 0 {
        chrono::Utc::now().year()
    } else {
        album.release_year
    };
    let relative_path = album_location(&primary.name, &album.title, release_year);
    let destination = music_dir.join(&relative_path);

    if destination.exists() {
        error!("Download destination {relative_path} already exists on disk");
        return Err(DownloadError::DestinationExists);
    }

    let staging_root = music_dir.join(STAGING_DIRECTORY);
    fs::create_dir_all(&staging_root).await.map_err(|e| {
        error!("Could not create staging directory: {e:#?}");
        DownloadError::Transfer
    })?;
    let staging_dir = staging_root.join(format!("{}-{}", album.id, nanoid::nanoid!()));
    fs::create_dir_all(&staging_dir).await.map_err(|e| {
        error!(
            "Could not create staging directory for album {}: {e:#?}",
            album.id
        );
        DownloadError::Transfer
    })?;

    let publish = async {
        downloader
            .download_album(&album, &staging_dir)
            .await
            .map_err(|e| {
                error!("Download failed: {e:#?}");
                DownloadError::Transfer
            })?;

        fs::create_dir_all(&destination).await.map_err(|e| {
            error!("Could not create parent directory for {relative_path}: {e:#?}");
            DownloadError::Transfer
        })?;
        fs::rename(&staging_dir, &destination).await.map_err(|e| {
            error!("Could not publish album to {relative_path}: {e:#?}");
            DownloadError::Transfer
        })?;
        Ok::<(), DownloadError>(())
    }
    .await;

    if publish.is_err() {
        let _ = fs::remove_dir_all(&staging_dir).await;
        return publish;
    }

    // The location is recorded only after publication succeeded, so database
    // state never claims an album the filesystem does not hold.
    let mut active: album::ActiveModel = album.into();
    active.relative_path = Set(Some(relative_path));
    active.update(db).await.map_err(|e| {
        error!("Db error saving Album location: {e:#?}");
        DownloadError::Save
    })?;

    Ok(())
}

type DeletionApiError = (StatusCode, String);

fn deletion_database_error(error: sea_orm::DbErr, files_removed: bool) -> DeletionApiError {
    tracing::error!(%error, files_removed, "Album deletion database operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        if files_removed {
            "Album files were removed, but catalog deletion failed. The album remains in the catalog."
        } else {
            "Could not delete the album from the catalog. No files were removed."
        }.to_owned(),
    )
}

pub(crate) async fn deletion_preview(
    State(state): State<AppState>,
    Path(album_id): Path<String>,
) -> Result<Json<dto::AlbumDeletionPreview>, DeletionApiError> {
    let album = album::Entity::find_by_id(album_id)
        .one(&state.db)
        .await
        .map_err(|error| deletion_database_error(error, false))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Album not found.".to_owned()))?;
    let absolute_path = album
        .relative_path
        .as_ref()
        .map(|relative_path| {
            path::absolute(state.queue.music_directory().path().join(relative_path))
                .map(|path| path.to_string_lossy().into_owned())
        })
        .transpose()
        .map_err(|error| {
            tracing::error!(%error, "Could not resolve album directory for deletion preview");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not resolve the album directory.".to_owned(),
            )
        })?;
    Ok(Json(dto::AlbumDeletionPreview { absolute_path }))
}

pub(crate) async fn delete(
    State(state): State<AppState>,
    Path(album_id): Path<String>,
    Json(request): Json<dto::AlbumDeletionRequest>,
) -> Result<Json<dto::AlbumDeletionResult>, DeletionApiError> {
    let transaction = state
        .db
        .begin()
        .await
        .map_err(|error| deletion_database_error(error, false))?;
    let album = album::Entity::find_by_id(&album_id)
        .one(&transaction)
        .await
        .map_err(|error| deletion_database_error(error, false))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Album not found.".to_owned()))?;
    let files_removed = if request.delete_files {
        let relative_path = album.relative_path.ok_or_else(|| {
                (StatusCode::UNPROCESSABLE_ENTITY, "Cannot delete files without a recorded album location. The catalog was not changed.".to_owned())
            })?;
        match remove_album_directory(state.queue.music_directory().path(), &relative_path).await {
            Ok(removed) => removed,
            Err(error) => {
                tracing::error!(?error, "Album directory removal failed");
                let _ = transaction.rollback().await;
                return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Could not remove the album directory. Files may have been partially removed. The catalog was not changed.".to_owned(),
                    ));
            }
        }
    } else {
        false
    };
    let catalog_result = async {
        let credits = album_artist::Entity::find()
            .filter(album_artist::Column::AlbumId.eq(&album_id))
            .all(&transaction)
            .await?;
        album_artist::Entity::delete_many()
            .filter(album_artist::Column::AlbumId.eq(&album_id))
            .exec(&transaction)
            .await?;
        album::Entity::delete_by_id(&album_id)
            .exec(&transaction)
            .await?;
        let mut removed_artist_ids = Vec::new();
        for credit in credits {
            if album_artist::Entity::find()
                .filter(album_artist::Column::ArtistId.eq(&credit.artist_id))
                .one(&transaction)
                .await?
                .is_none()
            {
                artist::Entity::delete_by_id(&credit.artist_id)
                    .exec(&transaction)
                    .await?;
                removed_artist_ids.push(credit.artist_id);
            }
        }
        Ok::<_, sea_orm::DbErr>(removed_artist_ids)
    }
    .await;
    let removed_artist_ids = match catalog_result {
        Ok(ids) => ids,
        Err(error) => {
            let _ = transaction.rollback().await;
            return Err(deletion_database_error(error, files_removed));
        }
    };
    transaction
        .commit()
        .await
        .map_err(|error| deletion_database_error(error, files_removed))?;
    Ok(Json(dto::AlbumDeletionResult { removed_artist_ids }))
}
