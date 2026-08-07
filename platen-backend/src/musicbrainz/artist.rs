use serde::Deserialize;
use tracing::instrument;

use super::release::{Area, Alias};
use super::{Musicbrainz, BASE_URL, RequestError};
impl Musicbrainz {
    #[instrument]
    pub async fn get_artist(&self, artist_id: &str) -> Result<Artist, RequestError> {
        let url = format!("{BASE_URL}/artist/{artist_id}?inc=aliases&fmt=json");
        let resp = self.fetch_with_retry(&url).await?;
        if !resp.status().is_success() {
            return Err(RequestError::MusicbrainzError(resp.status(), resp.text().await?))
        }
        Ok(resp.json().await?)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub sort_name: Option<String>,
    pub disambiguation: Option<String>,
    pub country: Option<String>,
    #[serde(rename = "type")]
    pub artist_type: Option<String>,
    pub type_id: Option<String>,
    pub gender: Option<String>,
    pub gender_id: Option<String>,
    pub life_span: Option<LifeSpan>,
    pub area: Option<Area>,
    pub begin_area: Option<Area>,
    pub end_area: Option<Area>,
    pub aliases: Option<Vec<Alias>>,
    pub isnis: Vec<String>,
    pub ipis: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LifeSpan {
    pub begin: Option<String>,
    pub end: Option<String>,
    pub ended: bool,
}
