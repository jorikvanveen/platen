use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};

use reqwest::StatusCode;
use serde::Deserialize;
use tracing::{error, info};

use crate::{
    app::AppState,
    entity::{album, album_artist},
    services,
};

use crate::routes::utils::map_tidal_error;

pub mod dto {
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct TidalArtist {
        pub id: String,
        pub name: String,
        pub profile_image_url: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct TidalAlbum {
        pub id: String,
        pub title: String,
        pub cover_url: Option<String>,
        pub album_type: String,
        pub release_date: Option<String>,
        pub popularity: f64,
        pub explicit: Option<bool>,
        pub media_tags: Option<Vec<String>>,
        pub available_quality: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct TidalArtistAlbums {
        pub artist: TidalArtist,
        pub albums: Vec<TidalAlbum>,
        pub returned_count: usize,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct TidalAlbumSearchResults {
        pub albums: Vec<TidalAlbumSearchHit>,
        pub returned_count: usize,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct TidalAlbumSearchHit {
        pub id: String,
        pub title: String,
        pub cover_url: Option<String>,
        pub album_type: String,
        pub release_date: Option<String>,
        pub popularity: f64,
        pub artists: Vec<TidalArtist>,
        pub explicit: Option<bool>,
        pub media_tags: Option<Vec<String>>,
        pub available_quality: Option<String>,
    }
}

impl From<services::tidal::TidalArtist> for dto::TidalArtist {
    fn from(a: services::tidal::TidalArtist) -> Self {
        dto::TidalArtist {
            id: a.id,
            name: a.name,
            profile_image_url: a.profile_image_url,
        }
    }
}

impl From<services::tidal::TidalAlbum> for dto::TidalAlbum {
    fn from(a: services::tidal::TidalAlbum) -> Self {
        dto::TidalAlbum {
            available_quality: services::tidal::available_quality(a.media_tags.as_deref())
                .map(str::to_owned),
            explicit: a.explicit,
            media_tags: a.media_tags,
            id: a.id,
            title: a.title,
            cover_url: a.cover_url,
            album_type: a.r#type,
            release_date: a.release_date,
            popularity: a.popularity,
        }
    }
}

impl From<services::tidal::ResolvedTidalSearchedAlbum> for dto::TidalAlbumSearchHit {
    fn from(a: services::tidal::ResolvedTidalSearchedAlbum) -> Self {
        dto::TidalAlbumSearchHit {
            available_quality: services::tidal::available_quality(a.media_tags.as_deref())
                .map(str::to_owned),
            explicit: a.explicit,
            media_tags: a.media_tags,
            id: a.id,
            title: a.title,
            cover_url: a.cover_url,
            album_type: a.r#type,
            release_date: a.release_date,
            popularity: a.popularity,
            artists: a.artists.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub query: String,
}

#[axum::debug_handler]
pub async fn search_artists(
    State(AppState { tidal, .. }): State<AppState>,
    Query(SearchQuery { query }): Query<SearchQuery>,
) -> Result<Json<Vec<dto::TidalArtist>>, StatusCode> {
    info!("Searching tidal for artist: {query}");
    let artists = tidal
        .search_artists(&query)
        .await
        .map_err(map_tidal_error)?;
    Ok(Json(artists.into_iter().map(Into::into).collect()))
}

#[axum::debug_handler]
pub async fn search_albums(
    State(AppState { tidal, db, .. }): State<AppState>,
    Query(SearchQuery { query }): Query<SearchQuery>,
) -> Result<Json<dto::TidalAlbumSearchResults>, StatusCode> {
    info!("Searching tidal for album: {query}");
    let albums = tidal.find_album(&query).await.map_err(map_tidal_error)?;
    let returned_count = albums.len();
    let albums = exclude_catalog_albums(&db, albums).await?;

    Ok(Json(dto::TidalAlbumSearchResults {
        albums: albums.into_iter().map(Into::into).collect(),
        returned_count,
    }))
}

#[axum::debug_handler]
pub async fn get_artist_albums(
    State(AppState { tidal, db, .. }): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<dto::TidalArtistAlbums>, StatusCode> {
    info!("Fetching tidal artist {id} with albums");
    let (artist, albums) = tokio::try_join!(tidal.get_artist(&id), tidal.get_artist_albums(&id))
        .map_err(map_tidal_error)?;
    let returned_count = albums.len();
    let albums = select_discovery_albums(&db, &id, albums).await?;
    Ok(Json(dto::TidalArtistAlbums {
        artist: artist.into(),
        albums: albums.into_iter().map(Into::into).collect(),
        returned_count,
    }))
}

async fn select_discovery_albums(
    db: &impl ConnectionTrait,
    artist_id: &str,
    candidate_albums: Vec<services::tidal::TidalAlbum>,
) -> Result<Vec<services::tidal::TidalAlbum>, StatusCode> {
    // Read all credited catalog titles, including editions absent from Tidal's response.
    let normalized_catalog_titles: HashSet<String> = album::Entity::find()
        .select_only()
        .column(album::Column::Title)
        .inner_join(album_artist::Entity)
        .filter(album_artist::Column::ArtistId.eq(artist_id))
        .into_tuple::<String>()
        .all(db)
        .await
        .map_err(|error| {
            error!("Could not check catalog artist titles: {error:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .iter()
        .map(|title| title.trim().to_lowercase())
        .collect();
    let unowned_candidates = candidate_albums
        .into_iter()
        .filter(|album| !normalized_catalog_titles.contains(&album.title.trim().to_lowercase()));
    deduplicate_discovery_albums(unowned_candidates)
}

fn discovery_rank(album: &services::tidal::TidalAlbum) -> Result<(u8, u8, u64), StatusCode> {
    let explicitness_rank = match album.explicit {
        Some(true) => 2,
        None => 1,
        Some(false) => 0,
    };
    let quality_rank = album
        .media_tags
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|tag| match tag.as_str() {
            "HIRES_LOSSLESS" => 3,
            "LOSSLESS" => 2,
            "DOLBY_ATMOS" => 1,
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    let numeric_album_id = album.id.parse::<u64>().map_err(|error| {
        error!(album_id = %album.id, "Invalid numeric Tidal album ID: {error}");
        StatusCode::BAD_GATEWAY
    })?;
    Ok((explicitness_rank, quality_rank, numeric_album_id))
}

fn deduplicate_discovery_albums(
    candidate_albums: impl IntoIterator<Item = services::tidal::TidalAlbum>,
) -> Result<Vec<services::tidal::TidalAlbum>, StatusCode> {
    let mut selected_index_by_title_and_type = HashMap::new();
    let mut selected_albums: Vec<services::tidal::TidalAlbum> = Vec::new();
    let mut selected_album_ranks = Vec::new();
    for candidate in candidate_albums {
        let title_and_type = (
            candidate.title.trim().to_lowercase(),
            candidate.r#type.clone(),
        );
        let candidate_rank = discovery_rank(&candidate)?;
        if let Some(&selected_index) = selected_index_by_title_and_type.get(&title_and_type) {
            if candidate_rank > selected_album_ranks[selected_index] {
                selected_albums[selected_index] = candidate;
                selected_album_ranks[selected_index] = candidate_rank;
            }
        } else {
            selected_index_by_title_and_type.insert(title_and_type, selected_albums.len());
            selected_albums.push(candidate);
            selected_album_ranks.push(candidate_rank);
        }
    }
    Ok(selected_albums)
}

trait HasAlbumId {
    fn album_id(&self) -> &str;
}

impl HasAlbumId for services::tidal::TidalAlbum {
    fn album_id(&self) -> &str {
        &self.id
    }
}

impl HasAlbumId for services::tidal::ResolvedTidalSearchedAlbum {
    fn album_id(&self) -> &str {
        &self.id
    }
}

async fn exclude_catalog_albums<T: HasAlbumId>(
    db: &impl ConnectionTrait,
    albums: Vec<T>,
) -> Result<Vec<T>, StatusCode> {
    if albums.is_empty() {
        return Ok(albums);
    }
    let album_ids: Vec<_> = albums.iter().map(HasAlbumId::album_id).collect();
    let catalog_ids: HashSet<String> = album::Entity::find()
        .select_only()
        .column(album::Column::Id)
        .filter(album::Column::Id.is_in(album_ids))
        .into_tuple::<String>()
        .all(db)
        .await
        .map_err(|error| {
            error!("Could not check catalog album membership: {error:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .into_iter()
        .collect();
    Ok(albums
        .into_iter()
        .filter(|album| !catalog_ids.contains(album.album_id()))
        .collect())
}

#[cfg(test)]
#[path = "../tests/routes/tidal.rs"]
mod tests;
