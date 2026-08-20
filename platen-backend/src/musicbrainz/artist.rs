use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::release_group::{Alias, Area};
use super::{BASE_URL, Musicbrainz, RequestError};
impl Musicbrainz {
    #[instrument]
    pub async fn get_artist(&self, artist_id: &str) -> Result<Artist, RequestError> {
        let url = format!("{BASE_URL}/artist/{artist_id}?inc=aliases&fmt=json");
        let resp = self.fetch_with_retry(&url).await?;
        if !resp.status().is_success() {
            return Err(RequestError::MusicbrainzError(
                resp.status(),
                resp.text().await?,
            ));
        }
        Ok(resp.json().await?)
    }

    #[instrument]
    pub async fn search_artist(
        &self,
        query: &str,
    ) -> Result<Vec<ArtistSearchResult>, RequestError> {
        let url = format!(
            "{BASE_URL}/artist?query={}&fmt=json",
            urlencoding::encode(query)
        );
        let resp = self.fetch_with_retry(&url).await?;
        if !resp.status().is_success() {
            return Err(RequestError::MusicbrainzError(
                resp.status(),
                resp.text().await?,
            ));
        }
        let resp: ArtistSearchResponse = resp.json().await?;
        Ok(resp.artists)
    }

    #[instrument]
    pub async fn get_release_groups(
        &self,
        artist_id: &str,
        page: usize,
    ) -> Result<ReleaseGroupResponse, RequestError> {
        let url = format!(
            "{BASE_URL}/release-group/?artist={artist_id}&limit=100&offset={}&fmt=json",
            page * 100
        );
        tracing::info!("URL: {}", url);
        let resp = self.fetch_with_retry(&url).await?;
        if !resp.status().is_success() {
            return Err(RequestError::MusicbrainzError(
                resp.status(),
                resp.text().await?,
            ));
        }
        let resp: ReleaseGroupResponse = resp.json().await?;
        Ok(resp)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct ArtistSearchResponse {
    artists: Vec<ArtistSearchResult>,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct ArtistSearchResult {
    pub id: String,
    pub name: String,
    pub country: Option<String>,
    pub disambiguation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub sort_name: Option<String>,
    pub disambiguation: Option<String>,
    pub country: Option<String>,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct LifeSpan {
    pub begin: Option<String>,
    pub end: Option<String>,
    pub ended: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct ReleaseGroupResponse {
    pub release_group_count: usize,
    pub release_groups: Vec<ReleaseGroup>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct ReleaseGroup {
    pub primary_type: String,
    pub disambiguation: Option<String>,
    pub id: String,
    pub first_release_date: Option<String>,
    pub title: String,
}
