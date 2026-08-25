use serde::Deserialize;
use tracing::instrument;

use super::{BASE_URL, RequestError};
use crate::services::musicbrainz::Musicbrainz;

impl Musicbrainz {
    #[instrument]
    pub async fn get_release_group(
        &self,
        release_group_id: &str,
    ) -> Result<ReleaseGroup, RequestError> {
        tracing::info!("Getting release group: {release_group_id}");
        let url =
            format!("{BASE_URL}/release-group/{release_group_id}?inc=artist-credits&fmt=json");
        let resp = self.fetch_with_retry(&url).await?;
        if !resp.status().is_success() {
            return Err(RequestError::MusicbrainzError(
                resp.status(),
                resp.text().await?,
            ));
        }
        Ok(resp.json().await?)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
#[allow(dead_code)]
pub struct ReleaseGroup {
    pub id: String,
    pub primary_type: String,
    pub title: String,
    pub first_release_date: Option<String>,
    #[serde(default)]
    pub artist_credit: Option<Vec<ArtistCredit>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
#[allow(dead_code)]
pub struct ArtistCredit {
    pub name: String,
    pub artist: ArtistCreditArtist,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
#[allow(unused)]
pub struct ArtistCreditArtist {
    pub id: String,
    pub name: String,
}
