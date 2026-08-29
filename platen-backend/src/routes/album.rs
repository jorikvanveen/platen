use std::path::PathBuf;

use chrono::{Datelike, NaiveDate};

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
    AppState,
    entity::{self, album, album_artist, artist},
    routes::artist::dto::Artist,
    services::{catalog_utils, downloaders::Downloader},
};

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
        pub album_type: Option<String>,
        pub release_year: i32,
        pub release_month: Option<i32>,
        pub release_day: Option<i32>,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct ReleaseDateRefreshSummary {
        pub updated: u32,
        pub skipped: u32,
    }
}

fn to_dto(model: album::Model, artists: Vec<Artist>) -> dto::Album {
    dto::Album {
        id: model.id,
        artists,
        title: model.title,
        album_type: model.album_type,
        release_year: model.release_year,
        release_month: model.release_month,
        release_day: model.release_day,
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
        let artists = credited_artists(&db, &existing.id)
            .await
            .map_err(|e| {
                error!("Db error: {e:#?}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        return Ok(Json(to_dto(existing, artists)));
    }

    let tidal_album = tidal
        .get_album(&album_id)
        .await
        .map_err(crate::routes::utils::map_tidal_error)?;
    let tidal_artists = tidal
        .get_album_artists(&album_id)
        .await
        .map_err(crate::routes::utils::map_tidal_error)?;

    let release_date = tidal_album
        .release_date
        .as_deref()
        .and_then(|date| parse_release_date(date).ok())
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    let new_album = album::ActiveModel {
        id: ActiveValue::Set(tidal_album.id),
        title: ActiveValue::Set(tidal_album.title),
        album_type: ActiveValue::Set(Some(tidal_album.r#type)),
        release_year: ActiveValue::Set(release_date.year),
        release_month: ActiveValue::Set(release_date.month),
        release_day: ActiveValue::Set(release_date.day),
    };
    let artists: Vec<Artist> = tidal_artists
        .iter()
        .map(|a| Artist {
            id: a.id.clone(),
            name: a.name.clone(),
        })
        .collect();

    let model = db
        .transaction::<_, album::Model, sea_orm::DbErr>(|txn| {
            Box::pin(async move {
                let model = new_album.insert(txn).await?;
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

    Ok(Json(to_dto(model, artists)))
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
    use super::{ReleaseDate, parse_release_date};

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
        .order_by_asc(album_artist::Column::Position)
        .all(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut result = Vec::with_capacity(rows.len());
    for (_, album_model) in rows {
        let Some(album_model) = album_model else {
            continue;
        };
        let artists = credited_artists(&db, &album_model.id).await.map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        result.push(to_dto(album_model, artists));
    }

    Ok(Json(result))
}

/// Legacy artist-scoped download path, kept so existing clients keep working.
/// The destination still comes from the primary artist; `artist_id` only
/// guards that the album's primary artist matches the path.
#[axum::debug_handler]
pub async fn download_artist_scoped(
    state: State<AppState>,
    Path((artist_id, album_id)): Path<(String, String)>,
) -> Result<(), StatusCode> {
    let artists = credited_artists(&state.db, &album_id)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let primary = artists.first().ok_or(StatusCode::NOT_FOUND)?;
    if primary.id != artist_id {
        return Err(StatusCode::NOT_FOUND);
    }
    download(state, Path(album_id)).await
}

#[axum::debug_handler]
pub async fn download(
    State(AppState {
        antra, db, config, ..
    }): State<AppState>,
    Path(album_id): Path<String>,
) -> Result<(), StatusCode> {
    let album = album::Entity::find_by_id(&album_id)
        .one(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let artists = credited_artists(&db, &album.id).await.map_err(|e| {
        error!("Db error: {e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let primary = artists
        .first()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    // The library layout is derived from the catalog for every release type,
    // never from the archive's own structure (ADR 0003). A release year of 0
    // means the date has not been refreshed from Tidal yet; the current year
    // keeps the directory from being named "(0)" in the meantime.
    let music_dir = PathBuf::from(config.music_dir);
    let release_year = if album.release_year == 0 {
        chrono::Utc::now().year()
    } else {
        album.release_year
    };
    let album_directory = format!("{} ({})", album.title, release_year);
    let destination = music_dir.join(&primary.name).join(album_directory);

    antra
        .download_album(&album, &destination)
        .await
        .map_err(|e| {
            error!("Download failed: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(())
}
