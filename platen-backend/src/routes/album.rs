use std::path::PathBuf;

use chrono::{Datelike, NaiveDate};

use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::StatusCode;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, ModelTrait, QueryFilter, Set,
};
use tracing::{error, info};

use crate::{
    AppState,
    entity::{self, album},
    services::downloaders::Downloader,
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

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct Album {
        pub id: String,
        pub artist_id: String,
        pub title: String,
        pub album_type: Option<String>,
        pub jellyfin_id: Option<String>,
        pub musicbrainz_release_group_id: Option<String>,
        pub match_method: Option<String>,
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

impl From<album::Model> for dto::Album {
    fn from(model: album::Model) -> Self {
        dto::Album {
            id: model.id,
            artist_id: model.artist_id,
            title: model.title,
            album_type: model.album_type,
            jellyfin_id: model.jellyfin_id,
            musicbrainz_release_group_id: model.musicbrainz_release_group_id,
            match_method: model.match_method,
            release_year: model.release_year,
            release_month: model.release_month,
            release_day: model.release_day,
        }
    }
}

#[axum::debug_handler]
pub async fn create(
    State(AppState { tidal, db, .. }): State<AppState>,
    Path((artist_id, album_id)): Path<(String, String)>,
) -> Result<Json<dto::Album>, StatusCode> {
    info!("Creating album {album_id} on artist {artist_id}");
    if let Some(existing) = album::Entity::find_by_id(&album_id)
        .one(&db)
        .await
        .map_err(|e| {
            error!("Db error: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    {
        return Ok(Json(existing.into()));
    }

    let tidal_album = tidal
        .get_album(&album_id)
        .await
        .map_err(crate::routes::utils::map_tidal_error)?;

    let release_date = tidal_album
        .release_date
        .as_deref()
        .and_then(|date| parse_release_date(date).ok())
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    let model = album::ActiveModel {
        id: ActiveValue::Set(tidal_album.id),
        artist_id: ActiveValue::Set(artist_id),
        title: ActiveValue::Set(tidal_album.title),
        album_type: ActiveValue::Set(Some(tidal_album.r#type)),
        jellyfin_id: ActiveValue::Set(None),
        musicbrainz_release_group_id: ActiveValue::Set(None),
        match_method: ActiveValue::Set(Some("tidal_id".into())),
        release_year: ActiveValue::Set(release_date.year),
        release_month: ActiveValue::Set(release_date.month),
        release_day: ActiveValue::Set(release_date.day),
    }
    .insert(&db)
    .await
    .map_err(|e| {
        error!("Db error: {e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(model.into()))
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

    let music_dir = PathBuf::from(config.music_dir);
    let destination = if album
        .album_type
        .as_deref()
        .is_some_and(|album_type| album_type.eq_ignore_ascii_case("SINGLE"))
    {
        let release_year = if album.release_year == 0 {
            chrono::Utc::now().year()
        } else {
            album.release_year
        };
        let album_directory = format!("{} ({})", album.title, release_year);
        music_dir.join(&artist.name).join(album_directory)
    } else {
        music_dir
    };

    antra
        .download_album(&album, &destination)
        .await
        .map_err(|e| {
            error!("Download failed: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(())
}
