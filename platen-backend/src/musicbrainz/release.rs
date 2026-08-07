use serde::Deserialize;
use tracing::instrument;

use crate::musicbrainz::Musicbrainz;
use super::{BASE_URL, RequestError};

impl Musicbrainz {
    #[instrument]
    pub async fn get_release(&self, release_id: &str) -> Result<Release, RequestError> {
        let url = format!("{BASE_URL}/release/{release_id}?inc=aliases%2Bartist-credits%2Blabels%2Bdiscids%2Brecordings&fmt=json");
        let resp = self.fetch_with_retry(&url).await?;
        if !resp.status().is_success() {
            return Err(RequestError::MusicbrainzError(resp.status(), resp.text().await?))
        }
        Ok(resp.json().await?)
    }
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Release {
    pub id: String,
    pub title: String,
    pub status: Option<String>,
    pub status_id: Option<String>,
    pub country: Option<String>,
    pub barcode: Option<String>,
    pub asin: Option<String>,
    pub date: Option<String>,
    pub quality: Option<String>,
    pub disambiguation: Option<String>,
    pub packaging: Option<String>,
    pub packaging_id: Option<String>,
    pub text_representation: Option<TextRepresentation>,
    pub cover_art_archive: Option<CoverArtArchive>,
    pub artist_credit: Vec<ArtistCredit>,
    pub label_info: Vec<LabelInfo>,
    pub release_events: Vec<ReleaseEvent>,
    pub aliases: Option<Vec<Alias>>,
    pub media: Vec<Media>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct ArtistCredit {
    pub joinphrase: Option<String>,
    pub name: String,
    pub artist: Artist,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub sort_name: Option<String>,
    pub country: Option<String>,
    #[serde(rename = "type")]
    pub artist_type: Option<String>,
    pub type_id: Option<String>,
    pub disambiguation: Option<String>,
    pub aliases: Option<Vec<Alias>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Alias {
    pub name: Option<String>,
    pub sort_name: Option<String>,
    pub locale: Option<String>,
    #[serde(rename = "type")]
    pub alias_type: Option<String>,
    pub type_id: Option<String>,
    pub primary: Option<bool>,
    pub begin: Option<String>,
    pub end: Option<String>,
    pub ended: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TextRepresentation {
    pub script: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CoverArtArchive {
    pub artwork: bool,
    pub count: u32,
    pub back: bool,
    pub front: bool,
    pub darkened: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LabelInfo {
    pub catalog_number: Option<String>,
    pub label: Option<Label>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Label {
    pub id: Option<String>,
    pub name: Option<String>,
    pub sort_name: Option<String>,
    pub disambiguation: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ReleaseEvent {
    pub date: Option<String>,
    pub area: Option<Area>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Area {
    pub id: String,
    pub name: String,
    pub sort_name: Option<String>,
    pub disambiguation: Option<String>,
    #[serde(rename = "type")]
    pub area_type: Option<String>,
    pub type_id: Option<String>,
    pub iso_3166_1_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Media {
    pub id: Option<String>,
    pub position: u32,
    pub track_count: u32,
    pub track_offset: Option<u32>,
    pub title: Option<String>,
    pub format: Option<String>,
    pub format_id: Option<String>,
    pub discs: Vec<Disc>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Disc {
    pub id: Option<String>,
    pub offset_count: Option<u32>,
    pub sectors: Option<u32>,
    pub offsets: Option<Vec<u32>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Track {
    pub id: String,
    pub number: String,
    pub position: u32,
    pub title: Option<String>,
    pub length: Option<u32>,
    pub artist_credit: Vec<ArtistCredit>,
    pub recording: Recording,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Recording {
    pub id: String,
    pub title: Option<String>,
    pub length: Option<u32>,
    pub first_release_date: Option<String>,
    pub disambiguation: Option<String>,
    pub video: bool,
    pub aliases: Option<Vec<Alias>>,
    pub artist_credit: Vec<ArtistCredit>,
}
