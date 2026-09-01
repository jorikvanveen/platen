use std::path::PathBuf;

use chrono::{Datelike, NaiveDate};
use futures_util::{StreamExt, TryStreamExt, stream};

use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::StatusCode;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set,
    TransactionTrait,
};
use tracing::{error, info};

use crate::{
    app::AppState,
    entity::{self, album, album_artist, artist},
    routes::{artist::dto::Artist, download::dto::DownloadJob},
    services::{self, catalog_utils, downloaders::Downloader},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseDate {
    pub year: i32,
    pub month: Option<i32>,
    pub day: Option<i32>,
}

fn sanitize_filename_component(value: &str) -> String {
    let mut sanitized = String::new();
    for character in value.chars() {
        match character {
            '/' | '\\' => {
                if !sanitized.ends_with(" - ") {
                    sanitized.push_str(" - ");
                }
            }
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => {
                sanitized.push('_');
            }
            character if character.is_control() => sanitized.push('_'),
            character => sanitized.push(character),
        }
    }

    let sanitized = sanitized.trim().trim_end_matches([' ', '.']);
    if sanitized.is_empty() {
        "Unknown album".to_owned()
    } else {
        sanitized.to_owned()
    }
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
        pub downloaded: bool,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct ReleaseDateRefreshSummary {
        pub updated: u32,
        pub skipped: u32,
    }
}

impl From<(album::Model, Vec<Artist>)> for dto::Album {
    fn from((model, artists): (album::Model, Vec<Artist>)) -> Self {
        Self {
            id: model.id,
            artists,
            title: model.title,
            cover_url: model.cover_url,
            album_type: model.album_type,
            release_year: model.release_year,
            release_month: model.release_month,
            release_day: model.release_day,
            downloaded: model.downloaded,
        }
    }
}

async fn credited_artists(
    db: &sea_orm::DatabaseConnection,
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
    info!("Creating album {album_id}");
    if let Some(existing) = album::Entity::find_by_id(&album_id)
        .one(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    {
        let artists = credited_artists(&db, &existing.id).await.map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        return Ok(Json((existing, artists).into()));
    }

    let tidal_album = tidal
        .get_album(&album_id)
        .await
        .map_err(crate::routes::utils::map_tidal_error)?;
    let tidal_artists = tidal
        .get_album_artists(&album_id)
        .await
        .map_err(crate::routes::utils::map_tidal_error)?;
    let cover_url = match tidal.get_album_cover(&album_id).await {
        Ok(cover_url) => cover_url,
        Err(error_message) => {
            error!("Could not fetch cover for album {album_id}: {error_message:#?}");
            None
        }
    };

    let release_date = tidal_album
        .release_date
        .as_deref()
        .and_then(|date| parse_release_date(date).ok())
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    let new_album = album::ActiveModel {
        id: ActiveValue::Set(tidal_album.id),
        title: ActiveValue::Set(tidal_album.title),
        cover_url: ActiveValue::Set(cover_url),
        album_type: ActiveValue::Set(Some(tidal_album.r#type)),
        release_year: ActiveValue::Set(release_date.year),
        release_month: ActiveValue::Set(release_date.month),
        release_day: ActiveValue::Set(release_date.day),
        ..Default::default()
    };
    let album_id_for_txn = album_id.clone();
    let model = db
        .transaction::<_, album::Model, sea_orm::DbErr>(|txn| {
            Box::pin(async move {
                album::Entity::insert(new_album)
                    .on_conflict_do_nothing()
                    .exec(txn)
                    .await?;
                let model = album::Entity::find_by_id(&album_id_for_txn)
                    .one(txn)
                    .await?
                    .ok_or_else(|| {
                        sea_orm::DbErr::RecordNotFound(format!(
                            "Album {album_id_for_txn} was not found after insertion"
                        ))
                    })?;
                for tidal_artist in &tidal_artists {
                    catalog_utils::upsert_artist(txn, tidal_artist).await?;
                }
                catalog_utils::insert_credits(txn, &model.id, &tidal_artists).await?;
                Ok(model)
            })
        })
        .await
        .map_err(|e| {
            error!("Db error creating album transaction: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let artists = credited_artists(&db, &model.id).await.map_err(|e| {
        error!("Db error loading album credits: {e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json((model, artists).into()))
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
mod tests {
    use std::path::Path;

    use migration::MigratorTrait;
    use sea_orm::{ActiveModelTrait, Database, EntityTrait, Set};
    use thiserror::Error;

    use super::{
        DownloadError, ReleaseDate, credited_artists, download_with, parse_release_date,
        sanitize_filename_component,
    };
    use crate::{
        entity::{album, album_artist, artist},
        services::downloaders::Downloader,
    };

    #[derive(Debug, Error)]
    #[error("test download failed")]
    struct TestDownloadError;

    struct TestDownloader {
        result: Result<(), TestDownloadError>,
    }

    #[async_trait::async_trait]
    impl Downloader for TestDownloader {
        async fn download_album(
            &self,
            _album: &album::Model,
            _destination: &Path,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            match &self.result {
                Ok(()) => Ok(()),
                Err(_) => Err(Box::new(TestDownloadError)),
            }
        }
    }

    async fn test_database() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        migration::Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn insert_test_album(
        db: &sea_orm::DatabaseConnection,
        downloaded: Option<bool>,
    ) -> album::Model {
        let mut new_album = album::ActiveModel {
            id: Set("album-1".into()),
            title: Set("Test album".into()),
            album_type: Set(Some("SINGLE".into())),
            release_year: Set(2026),
            release_month: Set(None),
            release_day: Set(None),
            ..Default::default()
        };
        if let Some(downloaded) = downloaded {
            new_album.downloaded = Set(downloaded);
        }
        let album = new_album.insert(db).await.unwrap();
        artist::ActiveModel {
            id: Set("artist-1".into()),
            name: Set("Test artist".into()),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
        album_artist::ActiveModel {
            album_id: Set(album.id.clone()),
            artist_id: Set("artist-1".into()),
            position: Set(0),
        }
        .insert(db)
        .await
        .unwrap();
        album
    }

    #[test]
    fn sanitizes_path_separators_without_losing_readability() {
        assert_eq!(
            sanitize_filename_component("Speakerboxxx/The Love Below"),
            "Speakerboxxx - The Love Below"
        );
        assert_eq!(sanitize_filename_component("A\\B/C"), "A - B - C");
    }

    #[test]
    fn replaces_invalid_characters_and_falls_back_for_empty_names() {
        assert_eq!(
            sanitize_filename_component("A:B* C? D\" E< F> G|"),
            "A_B_ C_ D_ E_ F_ G_"
        );
        assert_eq!(sanitize_filename_component("...   "), "Unknown album");
    }

    #[test]
    fn parses_release_date_at_each_supported_precision() {
        assert_eq!(
            parse_release_date("2020"),
            Ok(ReleaseDate {
                year: 2020,
                month: None,
                day: None
            })
        );
        assert_eq!(
            parse_release_date("2020-05"),
            Ok(ReleaseDate {
                year: 2020,
                month: Some(5),
                day: None
            })
        );
        assert_eq!(
            parse_release_date("2020-05-17"),
            Ok(ReleaseDate {
                year: 2020,
                month: Some(5),
                day: Some(17)
            })
        );
    }

    #[test]
    fn rejects_invalid_release_dates() {
        for value in [
            "20",
            "2020-5",
            "2020-13",
            "2020-02-30",
            "2020-05-1",
            "2020-05-17T00:00:00Z",
        ] {
            assert!(
                parse_release_date(value).is_err(),
                "{value} should be invalid"
            );
        }
    }

    #[tokio::test]
    async fn album_dto_orders_credits_primary_first() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        migration::Migrator::up(&db, None).await.unwrap();

        let album = album::ActiveModel {
            id: Set("album-1".into()),
            title: Set("Shared Credit".into()),
            album_type: Set(Some("ALBUM".into())),
            release_year: Set(2026),
            release_month: Set(Some(8)),
            release_day: Set(Some(29)),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
        for (id, name) in [("primary", "Primary"), ("featured", "Featured")] {
            artist::ActiveModel {
                id: Set(id.into()),
                name: Set(name.into()),
                ..Default::default()
            }
            .insert(&db)
            .await
            .unwrap();
        }
        for (artist_id, position) in [("featured", 1), ("primary", 0)] {
            album_artist::ActiveModel {
                album_id: Set(album.id.clone()),
                artist_id: Set(artist_id.into()),
                position: Set(position),
            }
            .insert(&db)
            .await
            .unwrap();
        }

        let artists = credited_artists(&db, &album.id).await.unwrap();
        let dto: super::dto::Album = (album, artists).into();

        assert_eq!(
            dto.artists
                .into_iter()
                .map(|artist| artist.id)
                .collect::<Vec<_>>(),
            ["primary", "featured"]
        );
        assert!(!dto.downloaded);
    }

    #[tokio::test]
    async fn new_albums_default_to_not_downloaded() {
        let db = test_database().await;
        let album = insert_test_album(&db, None).await;

        assert!(!album.downloaded);
    }

    #[tokio::test]
    async fn successful_download_is_saved() {
        let db = test_database().await;
        insert_test_album(&db, None).await;
        let downloader = TestDownloader { result: Ok(()) };

        download_with(&db, "/tmp/music", &downloader, "album-1")
            .await
            .unwrap();

        let album = album::Entity::find_by_id("album-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(album.downloaded);
    }

    #[tokio::test]
    async fn failed_download_is_not_saved() {
        let db = test_database().await;
        insert_test_album(&db, None).await;
        let downloader = TestDownloader {
            result: Err(TestDownloadError),
        };

        assert_eq!(
            download_with(&db, "/tmp/music", &downloader, "album-1").await,
            Err(DownloadError::Transfer)
        );

        let album = album::Entity::find_by_id("album-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(!album.downloaded);
    }

    #[tokio::test]
    async fn downloaded_album_conflicts_without_starting_another_download() {
        let db = test_database().await;
        insert_test_album(&db, Some(true)).await;
        let downloader = TestDownloader { result: Ok(()) };

        assert_eq!(
            download_with(&db, "/tmp/music", &downloader, "album-1").await,
            Err(DownloadError::AlreadyDownloaded)
        );
    }

    #[tokio::test]
    async fn missing_primary_artist_fails_download() {
        let db = test_database().await;
        let album = insert_test_album(&db, None).await;
        album_artist::Entity::delete_by_id((album.id.clone(), "artist-1".to_owned()))
            .exec(&db)
            .await
            .unwrap();
        artist::ActiveModel {
            id: Set("artist-2".into()),
            name: Set("Secondary artist".into()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
        album_artist::ActiveModel {
            album_id: Set(album.id),
            artist_id: Set("artist-2".into()),
            position: Set(1),
        }
        .insert(&db)
        .await
        .unwrap();

        let downloader = TestDownloader { result: Ok(()) };

        assert_eq!(
            download_with(&db, "/tmp/music", &downloader, "album-1").await,
            Err(DownloadError::Catalog)
        );
    }
}

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
                Ok::<_, sea_orm::DbErr>((album_model, artists).into())
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
        })?;
    if album.as_ref().is_some_and(|album| album.downloaded) {
        return Err(StatusCode::CONFLICT);
    }
    let release_name = album.map(|album| album.title);
    let job = queue.enqueue(album_id).await.map_err(|error| match error {
        services::download_queue::QueueError::Full => StatusCode::TOO_MANY_REQUESTS,
        services::download_queue::QueueError::WorkerStopped => {
            error!("Could not enqueue download job: {error:#?}");
            StatusCode::SERVICE_UNAVAILABLE
        }
    })?;
    Ok((
        StatusCode::ACCEPTED,
        Json(DownloadJob::from_record(job, release_name)),
    ))
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DownloadError {
    #[error("could not load album metadata")]
    Catalog,
    #[error("album is already downloaded")]
    AlreadyDownloaded,
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

    if album.downloaded {
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
    let album_directory = format!(
        "{} ({})",
        sanitize_filename_component(&album.title),
        release_year
    );
    let destination = music_dir.join(&primary.name).join(album_directory);

    downloader
        .download_album(&album, &destination)
        .await
        .map_err(|e| {
            error!("Download failed: {e:#?}");
            DownloadError::Transfer
        })?;

    let mut active: album::ActiveModel = album.into();
    active.downloaded = Set(true);
    active.update(db).await.map_err(|e| {
        error!("Db error marking album as downloaded: {e:#?}");
        DownloadError::Save
    })?;

    Ok(())
}
