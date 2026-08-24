use axum::{
    Json,
    extract::{Path, Query, State},
};
use reqwest::StatusCode;
use serde::Deserialize;
use tracing::info;

use crate::AppState;

use crate::routes::utils::map_tidal_error;

pub mod dto {
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct TidalArtist {
        pub id: String,
        pub name: String,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct TidalAlbum {
        pub id: String,
        pub title: String,
        pub album_type: String,
        pub release_date: Option<String>,
        pub popularity: f64,
    }
}

impl From<crate::services::tidal::TidalArtist> for dto::TidalArtist {
    fn from(a: crate::services::tidal::TidalArtist) -> Self {
        dto::TidalArtist {
            id: a.id,
            name: a.name,
        }
    }
}

impl From<crate::services::tidal::TidalAlbum> for dto::TidalAlbum {
    fn from(a: crate::services::tidal::TidalAlbum) -> Self {
        dto::TidalAlbum {
            id: a.id,
            title: a.title,
            album_type: a.r#type,
            release_date: a.release_date,
            popularity: a.popularity,
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
    let artists = {
        let mut tidal = tidal.lock().await;
        tidal.search_artists(&query).await.map_err(map_tidal_error)?
    };
    Ok(Json(artists.into_iter().map(Into::into).collect()))
}

#[axum::debug_handler]
pub async fn get_artist_albums(
    State(AppState { tidal, .. }): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<dto::TidalAlbum>>, StatusCode> {
    info!("Fetching tidal artist {id} with albums");
    let albums = {
        let mut tidal = tidal.lock().await;
        tidal
            .get_artist_albums(&id)
            .await
            .map_err(map_tidal_error)?
    };
    Ok(Json(albums.into_iter().map(Into::into).collect()))
}
