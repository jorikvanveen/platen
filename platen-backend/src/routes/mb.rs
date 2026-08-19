use axum::{
    Json,
    extract::{Path, Query, State},
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::{
    AppState,
    musicbrainz::{self, artist::{ArtistSearchResult, ReleaseGroupResponse}},
};

#[axum::debug_handler]
pub async fn get_artist(
    State(AppState { musicbrainz, .. }): State<AppState>,
    Path(artist_id): Path<String>,
) -> Result<Json<musicbrainz::artist::Artist>, StatusCode> {
    info!("Fetching musicbrainz artist {artist_id}");

    let artist = musicbrainz
        .get_artist(&artist_id)
        .await
        .map_err(|e| match e {
            musicbrainz::RequestError::MusicbrainzError(StatusCode::NOT_FOUND, _) => {
                StatusCode::NOT_FOUND
            }
            e => {
                error!("{e:#?}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
    info!("Returned artist");
    Ok(Json(artist))
}

pub async fn search_artist(
    State(AppState { musicbrainz, .. }): State<AppState>,
    Path(query): Path<String>,
) -> Result<Json<Vec<ArtistSearchResult>>, StatusCode> {
    info!("Searching for artist: {query}");

    let result = musicbrainz.search_artist(&query).await.map_err(|e| {
        error!("{e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
pub struct Pagination {
    page: usize,
}

pub async fn get_artist_release_groups(
    State(AppState { musicbrainz, .. }): State<AppState>,
    Path(artist_id): Path<String>,
    Query(Pagination { page }): Query<Pagination>,
) -> Result<Json<ReleaseGroupResponse>, StatusCode> {
    info!("Getting release group page: {}", page);
    Ok(Json(
        musicbrainz
            .get_release_groups(&artist_id, page)
            .await
            .map_err(|e| {
                error!("{:#?}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?,
    ))
}
