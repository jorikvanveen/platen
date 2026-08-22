use axum::{
    Json,
    extract::{Path, Query, State},
};
use reqwest::StatusCode;
use tracing::{error, info};

use crate::{
    AppState,
    routes::utils::Pagination,
    services::musicbrainz::{self, artist, release_group},
};

pub mod dto {
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct ArtistSearchResponse {
        pub artist_count: usize,
        pub artists: Vec<ArtistSearchResult>,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct ArtistSearchResult {
        pub id: String,
        pub name: String,
        pub country: Option<String>,
        pub disambiguation: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct ReleaseGroupResponse {
        pub release_group_count: usize,
        pub release_groups: Vec<MbReleaseGroup>,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct MbReleaseGroup {
        pub primary_type: Option<String>,
        pub disambiguation: Option<String>,
        pub id: String,
        pub first_release_date: Option<String>,
        pub title: String,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct MbArtist {
        pub id: String,
        pub name: String,
        pub sort_name: Option<String>,
        pub disambiguation: Option<String>,
        pub country: Option<String>,
        #[ts(rename = "type")]
        pub r#type: Option<String>,
        pub type_id: Option<String>,
        pub gender: Option<String>,
        pub gender_id: Option<String>,
        pub life_span: Option<LifeSpan>,
        pub area: Option<Area>,
        pub begin_area: Option<Area>,
        pub end_area: Option<Area>,
        pub aliases: Option<Vec<Alias>>,
        pub isnis: Option<Vec<String>>,
        pub ipis: Option<Vec<String>>,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct LifeSpan {
        pub begin: Option<String>,
        pub end: Option<String>,
        pub ended: bool,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct Area {
        pub id: String,
        pub name: String,
        pub sort_name: Option<String>,
        pub disambiguation: Option<String>,
        pub area_type: Option<String>,
        pub type_id: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize, TS)]
    #[ts(export)]
    pub struct Alias {
        pub name: Option<String>,
        pub sort_name: Option<String>,
        pub locale: Option<String>,
        pub alias_type: Option<String>,
        pub type_id: Option<String>,
        pub primary: Option<bool>,
        pub begin: Option<String>,
        pub end: Option<String>,
        pub ended: Option<bool>,
    }
}

impl From<artist::ArtistSearchResponse> for dto::ArtistSearchResponse {
    fn from(resp: artist::ArtistSearchResponse) -> Self {
        dto::ArtistSearchResponse {
            artist_count: resp.artist_count,
            artists: resp.artists.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<artist::ArtistSearchResult> for dto::ArtistSearchResult {
    fn from(result: artist::ArtistSearchResult) -> Self {
        dto::ArtistSearchResult {
            id: result.id,
            name: result.name,
            country: result.country,
            disambiguation: result.disambiguation,
        }
    }
}

impl From<artist::ReleaseGroupResponse> for dto::ReleaseGroupResponse {
    fn from(resp: artist::ReleaseGroupResponse) -> Self {
        dto::ReleaseGroupResponse {
            release_group_count: resp.release_group_count,
            release_groups: resp.release_groups.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<artist::ReleaseGroup> for dto::MbReleaseGroup {
    fn from(group: artist::ReleaseGroup) -> Self {
        dto::MbReleaseGroup {
            primary_type: group.primary_type,
            disambiguation: group.disambiguation,
            id: group.id,
            first_release_date: group.first_release_date,
            title: group.title,
        }
    }
}

impl From<artist::Artist> for dto::MbArtist {
    fn from(artist: artist::Artist) -> Self {
        dto::MbArtist {
            id: artist.id,
            name: artist.name,
            sort_name: artist.sort_name,
            disambiguation: artist.disambiguation,
            country: artist.country,
            r#type: artist.r#type,
            type_id: artist.type_id,
            gender: artist.gender,
            gender_id: artist.gender_id,
            life_span: artist.life_span.map(Into::into),
            area: artist.area.map(Into::into),
            begin_area: artist.begin_area.map(Into::into),
            end_area: artist.end_area.map(Into::into),
            aliases: artist
                .aliases
                .map(|aliases| aliases.into_iter().map(Into::into).collect()),
            isnis: artist.isnis,
            ipis: artist.ipis,
        }
    }
}

impl From<artist::LifeSpan> for dto::LifeSpan {
    fn from(life_span: artist::LifeSpan) -> Self {
        dto::LifeSpan {
            begin: life_span.begin,
            end: life_span.end,
            ended: life_span.ended,
        }
    }
}

impl From<release_group::Alias> for dto::Alias {
    fn from(alias: release_group::Alias) -> Self {
        dto::Alias {
            name: alias.name,
            sort_name: alias.sort_name,
            locale: alias.locale,
            alias_type: alias.alias_type,
            type_id: alias.type_id,
            primary: alias.primary,
            begin: alias.begin,
            end: alias.end,
            ended: alias.ended,
        }
    }
}

impl From<release_group::Area> for dto::Area {
    fn from(area: release_group::Area) -> Self {
        dto::Area {
            id: area.id,
            name: area.name,
            sort_name: area.sort_name,
            disambiguation: area.disambiguation,
            area_type: area.area_type,
            type_id: area.type_id,
        }
    }
}

#[axum::debug_handler]
pub async fn get_artist(
    State(AppState { musicbrainz, .. }): State<AppState>,
    Path(artist_id): Path<String>,
) -> Result<Json<dto::MbArtist>, StatusCode> {
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
    Ok(Json(artist.into()))
}

pub async fn search_artist(
    State(AppState { musicbrainz, .. }): State<AppState>,
    Path(query): Path<String>,
    Query(Pagination { page }): Query<Pagination>,
) -> Result<Json<dto::ArtistSearchResponse>, StatusCode> {
    info!("Searching for artist: {query} (page {page})");

    let result = musicbrainz.search_artist(&query, page).await.map_err(|e| {
        error!("{e:#?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(result.into()))
}

pub async fn get_artist_release_groups(
    State(AppState { musicbrainz, .. }): State<AppState>,
    Path(artist_id): Path<String>,
    Query(Pagination { page }): Query<Pagination>,
) -> Result<Json<dto::ReleaseGroupResponse>, StatusCode> {
    info!("Getting release group page: {}", page);
    Ok(Json(
        musicbrainz
            .get_release_groups(&artist_id, page)
            .await
            .map_err(|e| {
                error!("{:#?}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .into(),
    ))
}
