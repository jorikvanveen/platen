use axum::{Json, extract::{Path, State}};
use reqwest::StatusCode;
use tracing::{error, info};

use crate::{AppState, musicbrainz::{self, artist::ArtistSearchResult}};

#[axum::debug_handler]
pub async fn get_artist(
    State(AppState { musicbrainz, .. }): State<AppState>,
    Path(artist_id): Path<String>
) -> Result<Json<musicbrainz::artist::Artist>, StatusCode> {
    info!("Fetching musicbrainz artist {artist_id}");
    
    let artist = musicbrainz.get_artist(&artist_id).await.map_err(|e| match e {
        musicbrainz::RequestError::MusicbrainzError(StatusCode::NOT_FOUND, _) => StatusCode::NOT_FOUND,
        e => {
            error!("{e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    })?;
    
    Ok(Json(artist))
}

pub async fn search_artist(
    State(AppState { musicbrainz, .. }): State<AppState>,
    Path(query): Path<String>
) -> Result<Json<Vec<ArtistSearchResult>>, StatusCode> {
    info!("Searching for artist: {query}");
    let result = musicbrainz.search_artist(&query).await.map_err(|e| {
        error!("{e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(result))
}
