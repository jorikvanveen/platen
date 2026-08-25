use base64::prelude::*;
use chrono::Utc;
use reqwest::{RequestBuilder, StatusCode};
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;

use tidal_response::{
    AlbumSearchIncludedAttributes, AlbumSearchRelationshipsAlbumsData, AlbumSingleResource,
    AlbumWithArtistsDocument, ArtistAlbumsRelationshipDocument, ArtistSearchDocument,
    ArtistSingleResource,
};

static TIDAL_BASE_URL: &str = "https://openapi.tidal.com/v2";

#[derive(Error, Debug)]
pub enum TidalError {
    #[error("Tidal API sent back an unexpected response")]
    UnexpectedResponse,

    #[error("Error sending request to tidal API: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Failed to authenticate with tidal API: {0}: {1}")]
    AuthenticationFailed(StatusCode, String),
}

pub struct Tidal {
    client: reqwest::Client,
    token: Option<String>,
    expires_at: chrono::DateTime<Utc>,
    client_id: String,
    client_secret: String,
}

impl Tidal {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client: reqwest::ClientBuilder::new().build().unwrap(),
            token: None,
            expires_at: Default::default(),
            client_id,
            client_secret,
        }
    }

    pub async fn login(&mut self) -> Result<(), TidalError> {
        self.ensure_token().await?;
        tracing::info!("Logged in to tidal");
        Ok(())
    }

    async fn send_with_retry(
        &mut self,
        builder: RequestBuilder,
    ) -> Result<reqwest::Response, TidalError> {
        const MAX_RETRIES: u32 = 3;
        let mut retries: u32 = 0;
        let mut retried_auth = false;

        loop {
            let token = self.ensure_token().await?;
            let req = builder
                .try_clone()
                .ok_or(TidalError::UnexpectedResponse)?
                .bearer_auth(token);

            match req.send().await {
                Ok(res)
                    if res.status() == StatusCode::TOO_MANY_REQUESTS && retries < MAX_RETRIES =>
                {
                    let wait_secs = res
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or_else(|| 1 << retries);

                    sleep(Duration::from_secs(wait_secs)).await;
                    retries += 1;
                }
                Ok(res) if res.status() == StatusCode::UNAUTHORIZED && !retried_auth => {
                    self.token = None;
                    retried_auth = true;
                }
                Ok(res) => return Ok(res),
                Err(_) if retries < MAX_RETRIES => {
                    sleep(Duration::from_secs(1 << retries)).await;
                    retries += 1;
                }
                Err(e) => return Err(TidalError::Reqwest(e)),
            }
        }
    }

    async fn get_oauth_token(&mut self) -> Result<(), TidalError> {
        let credentials =
            BASE64_STANDARD.encode(format!("{}:{}", self.client_id, self.client_secret));

        let response = self
            .client
            .post("https://auth.tidal.com/v1/oauth2/token")
            .header("Authorization", format!("Basic {}", credentials))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("grant_type=client_credentials")
            .send()
            .await?;

        if response.status() != StatusCode::OK {
            return Err(TidalError::AuthenticationFailed(
                response.status(),
                response.text().await?,
            ));
        }

        let response: tidal_response::OauthToken = response
            .json()
            .await
            .map_err(|_| TidalError::UnexpectedResponse)?;

        self.token = Some(response.access_token);
        self.expires_at =
            chrono::Utc::now() + Duration::from_secs(response.expires_in.saturating_sub(60));

        tracing::info!("Authenticated with tidal");

        Ok(())
    }

    async fn ensure_token(&mut self) -> Result<String, TidalError> {
        let now = chrono::Utc::now();

        if self.token.is_some() && self.expires_at > now {
            return Ok(self.token.clone().unwrap());
        }

        self.get_oauth_token().await?;
        Ok(self.token.clone().unwrap())
    }

    pub async fn find_album(
        &mut self,
        query: &str,
    ) -> Result<Vec<ResolvedTidalSearchedAlbum>, TidalError> {
        let release_query = urlencoding::encode(query);
        let url =
            format!("{TIDAL_BASE_URL}/searchResults?filter[query]={release_query}&include=albums");

        let resp = self.send_with_retry(self.client.get(url)).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let resp: tidal_response::AlbumSearch = resp.json().await?;

        resp.data
            .first()
            .ok_or(TidalError::UnexpectedResponse)?
            .relationships
            .albums
            .data
            .iter()
            .map(|relationship| {
                resp.included
                    .iter()
                    .find(|included| included.id == relationship.id)
                    .ok_or(TidalError::UnexpectedResponse)
                    .map(|found_include| (relationship, &found_include.attributes).into())
            })
            .collect()
    }

    /// `GET /artists/{id}` -> single artist resource.
    pub async fn get_artist(&mut self, id: &str) -> Result<TidalArtist, TidalError> {
        let url = format!("{TIDAL_BASE_URL}/artists/{id}");
        let resp = self.send_with_retry(self.client.get(url)).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let doc: ArtistSingleResource = resp.json().await?;
        let resource = doc.data.ok_or(TidalError::UnexpectedResponse)?;
        Ok(TidalArtist {
            id: resource.id,
            name: resource.attributes.name,
        })
    }

    /// `GET /albums/{id}` -> single album resource.
    pub async fn get_album(&mut self, id: &str) -> Result<TidalAlbum, TidalError> {
        let url = format!("{TIDAL_BASE_URL}/albums/{id}");
        let resp = self.send_with_retry(self.client.get(url)).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let doc: AlbumSingleResource = resp.json().await?;
        let resource = doc.data.ok_or(TidalError::UnexpectedResponse)?;
        Ok(TidalAlbum::from(resource.id, resource.attributes))
    }

    /// Exhausts `GET /artists/{id}/relationships/albums?include=albums` across
    /// Tidal's cursor pages, returning the full set of albums.
    ///
    /// `GET /artists/{id}?include=albums` exposes no paging parameter, so it only
    /// returns Tidal's default first page. The relationship endpoint pages via
    /// `links.next` (a relative path carrying an opaque `page[cursor]`). The loop
    /// follows `links.next` until it is absent, or after `MAX_PAGES` requests as
    /// a safety cap.
    pub async fn get_artist_albums(&mut self, id: &str) -> Result<Vec<TidalAlbum>, TidalError> {
        const MAX_PAGES: usize = 50;
        let mut albums: Vec<TidalAlbum> = Vec::new();
        let mut next: Option<String> = None;

        for page in 0..MAX_PAGES {
            let url = match &next {
                Some(rel) => format!("{TIDAL_BASE_URL}{rel}"),
                None => {
                    format!("{TIDAL_BASE_URL}/artists/{id}/relationships/albums?include=albums")
                }
            };
            let resp = self.send_with_retry(self.client.get(url)).await?;
            if !resp.status().is_success() {
                tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
                return Err(TidalError::UnexpectedResponse);
            }

            let doc: ArtistAlbumsRelationshipDocument = resp.json().await?;
            for inc in &doc.included {
                albums.push(TidalAlbum::from(inc.id.clone(), inc.attributes.clone()));
            }

            next = doc.links.and_then(|l| l.next);
            if next.is_none() {
                return Ok(albums);
            }
            tracing::debug!("tidal: fetching artist {id} albums page {}", page + 1);
        }

        tracing::warn!(
            "tidal: hit max pages ({MAX_PAGES}) exhausting artist {id} albums; \
             returning {} albums",
            albums.len()
        );
        Ok(albums)
    }

    /// `GET /albums/{id}?include=artists` -> album with its artists.
    pub async fn get_album_artists(&mut self, id: &str) -> Result<Vec<TidalArtist>, TidalError> {
        let url = format!("{TIDAL_BASE_URL}/albums/{id}?include=artists");
        let resp = self.send_with_retry(self.client.get(url)).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let doc: AlbumWithArtistsDocument = resp.json().await?;
        let album = doc.data.ok_or(TidalError::UnexpectedResponse)?;

        let relationships = album
            .relationships
            .and_then(|r| r.artists)
            .ok_or(TidalError::UnexpectedResponse)?;

        relationships
            .data
            .iter()
            .map(|rel| {
                doc.included
                    .iter()
                    .find(|inc| inc.id == rel.id)
                    .ok_or(TidalError::UnexpectedResponse)
                    .map(|inc| TidalArtist {
                        id: inc.id.clone(),
                        name: inc.attributes.name.clone(),
                    })
            })
            .collect()
    }

    /// `GET /searchResults?filter[query]=...&include=artists` -> artist search.
    pub async fn search_artists(&mut self, query: &str) -> Result<Vec<TidalArtist>, TidalError> {
        let encoded = urlencoding::encode(query);
        let url = format!("{TIDAL_BASE_URL}/searchResults?filter[query]={encoded}&include=artists");

        let resp = self.send_with_retry(self.client.get(url)).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let doc: ArtistSearchDocument = resp.json().await?;
        let search = doc.data.first().ok_or(TidalError::UnexpectedResponse)?;

        let relationships = search
            .relationships
            .as_ref()
            .and_then(|r| r.artists.as_ref())
            .ok_or(TidalError::UnexpectedResponse)?;

        relationships
            .data
            .iter()
            .map(|rel| {
                doc.included
                    .iter()
                    .find(|inc| inc.id == rel.id)
                    .ok_or(TidalError::UnexpectedResponse)
                    .map(|inc| TidalArtist {
                        id: inc.id.clone(),
                        name: inc.attributes.name.clone(),
                    })
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct TidalArtist {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub struct TidalAlbum {
    pub id: String,
    pub title: String,
    pub release_date: Option<String>,
    pub barcode_id: Option<String>,
    pub number_of_volumes: Option<u32>,
    pub number_of_items: Option<u32>,
    pub duration: String,
    pub explicit: bool,
    pub popularity: f64,
    pub availability: Option<Vec<String>>,
    pub media_tags: Option<Vec<String>>,
    pub r#type: String,
}

impl TidalAlbum {
    fn from(id: String, attr: AlbumSearchIncludedAttributes) -> Self {
        TidalAlbum {
            id,
            title: attr.title,
            release_date: attr.release_date,
            barcode_id: attr.barcode_id,
            number_of_volumes: attr.number_of_volumes,
            number_of_items: attr.number_of_items,
            duration: attr.duration,
            explicit: attr.explicit,
            popularity: attr.popularity,
            availability: attr.availability,
            media_tags: attr.media_tags,
            r#type: attr.r#type,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ResolvedTidalSearchedAlbum {
    pub id: String,
    pub title: String,
    pub barcode_id: Option<String>,
    pub number_of_volumes: Option<u32>,
    pub number_of_items: Option<u32>,
    pub duration: String, // ISO 8601
    pub explicit: bool,
    pub release_date: Option<String>, // 2022-04-20
    pub popularity: f64,
    pub access_type: Option<String>,
    pub availability: Option<Vec<String>>,
    pub media_tags: Option<Vec<String>>,
    pub r#type: String,
}

impl
    From<(
        &AlbumSearchRelationshipsAlbumsData,
        &AlbumSearchIncludedAttributes,
    )> for ResolvedTidalSearchedAlbum
{
    fn from(
        val: (
            &AlbumSearchRelationshipsAlbumsData,
            &AlbumSearchIncludedAttributes,
        ),
    ) -> Self {
        let (data, attr) = val;
        ResolvedTidalSearchedAlbum {
            id: data.id.clone(),
            title: attr.title.clone(),
            barcode_id: attr.barcode_id.clone(),
            number_of_volumes: attr.number_of_volumes,
            number_of_items: attr.number_of_items,
            duration: attr.duration.clone(),
            explicit: attr.explicit,
            release_date: attr.release_date.clone(),
            popularity: attr.popularity,
            access_type: attr.access_type.clone(),
            availability: attr.availability.clone(),
            media_tags: attr.media_tags.clone(),
            r#type: attr.r#type.clone(),
        }
    }
}

mod tidal_response {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct OauthToken {
        pub access_token: String,
        pub expires_in: u64,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSearch {
        pub data: Vec<AlbumSearchData>, // Should always be of length 1
        pub included: Vec<AlbumSearchIncluded>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct AlbumSearchData {
        pub id: String,
        pub r#type: String,
        pub relationships: AlbumSearchRelationships,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSearchRelationships {
        pub albums: AlbumSearchRelationshipsAlbums,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSearchRelationshipsAlbums {
        pub data: Vec<AlbumSearchRelationshipsAlbumsData>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct AlbumSearchRelationshipsAlbumsData {
        pub id: String,
        pub r#type: String, // always "albums"
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct AlbumSearchIncluded {
        pub id: String,
        pub r#type: String,
        pub attributes: AlbumSearchIncludedAttributes,
    }
    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct AlbumSearchIncludedAttributes {
        pub title: String,
        pub barcode_id: Option<String>,
        pub number_of_volumes: Option<u32>,
        pub number_of_items: Option<u32>,
        pub duration: String, // ISO 8601
        pub explicit: bool,
        pub release_date: Option<String>, // 2022-04-20
        pub popularity: f64,
        pub access_type: Option<String>,
        pub availability: Option<Vec<String>>,
        pub media_tags: Option<Vec<String>>,
        pub r#type: String,
    }

    #[derive(Debug, Deserialize)]
    pub struct ArtistSingleResource {
        pub data: Option<ArtistResource>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(unused)]
    pub struct ArtistResource {
        pub id: String,
        pub r#type: String,
        pub attributes: ArtistIncludedAttributes,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSingleResource {
        pub data: Option<AlbumResource>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(unused)]
    pub struct AlbumResource {
        pub id: String,
        pub r#type: String,
        pub attributes: AlbumSearchIncludedAttributes,
    }

    // ---- Compound documents (include=...) ----

    /// `GET /artists/{id}/relationships/albums?include=albums` response: the
    /// `data` array holds album resource identifiers, `included` holds the full
    /// album resources, and `links.next` is the relative path to the next cursor
    /// page (absent on the last page).
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct ArtistAlbumsRelationshipDocument {
        #[serde(default)]
        pub data: Vec<AlbumSearchRelationshipsAlbumsData>,
        #[serde(default)]
        pub included: Vec<AlbumSearchIncluded>,
        #[serde(default)]
        pub links: Option<Links>,
    }

    #[derive(Debug, Deserialize, Default)]
    #[serde(default)]
    pub struct Links {
        pub next: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumWithArtistsDocument {
        pub data: Option<AlbumWithArtistsResource>,
        #[serde(default)]
        pub included: Vec<ArtistIncluded>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(unused)]
    pub struct AlbumWithArtistsResource {
        pub id: String,
        pub r#type: String,
        pub attributes: AlbumSearchIncludedAttributes,
        pub relationships: Option<AlbumWithArtistsRelationships>,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumWithArtistsRelationships {
        pub artists: Option<AlbumSearchRelationshipsAlbums>,
    }

    // ---- Artist search ----

    #[derive(Debug, Deserialize)]
    pub struct ArtistSearchDocument {
        pub data: Vec<ArtistSearchData>,
        #[serde(default)]
        pub included: Vec<ArtistIncluded>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct ArtistSearchData {
        pub id: String,
        pub r#type: String,
        pub relationships: Option<ArtistSearchRelationships>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ArtistSearchRelationships {
        pub artists: Option<AlbumSearchRelationshipsAlbums>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct ArtistIncluded {
        pub id: String,
        pub r#type: String,
        pub attributes: ArtistIncludedAttributes,
    }

    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct ArtistIncludedAttributes {
        pub name: String,
    }
}

#[cfg(test)]
mod tests {
    use super::tidal_response::{AlbumSearchIncludedAttributes, ArtistAlbumsRelationshipDocument};

    /// Regression: `AlbumSearchIncludedAttributes` used `#[serde(rename =
    /// "camelCase")]`, which renames the type, not the fields. Serde looked for
    /// `release_date` in the JSON while Tidal sends `releaseDate`, so every
    /// snake_case field silently deserialized to `None`. The fix is `rename_all`.
    /// This test pins the camelCase field names Tidal actually sends.
    #[test]
    fn deserializes_camel_case_attributes() {
        let json = r#"
            {
              "title": "Michelle (Take 1)",
              "barcodeId": "00881061189336",
              "numberOfVolumes": 1,
              "numberOfItems": 1,
              "duration": "PT3M17S",
              "explicit": false,
              "releaseDate": "2026-07-29",
              "popularity": 0.7160302460678571,
              "accessType": "PUBLIC",
              "availability": ["STREAM", "DJ"],
              "mediaTags": ["HIRES_LOSSLESS", "LOSSLESS"],
              "type": "SINGLE"
            }
        "#;

        let attr: AlbumSearchIncludedAttributes = serde_json::from_str(json).unwrap();

        assert_eq!(attr.title, "Michelle (Take 1)");
        assert_eq!(attr.barcode_id.as_deref(), Some("00881061189336"));
        assert_eq!(attr.number_of_volumes, Some(1));
        assert_eq!(attr.number_of_items, Some(1));
        assert_eq!(attr.duration, "PT3M17S");
        assert!(!attr.explicit);
        assert_eq!(attr.release_date.as_deref(), Some("2026-07-29"));
        assert!((attr.popularity - 0.7160302460678571).abs() < f64::EPSILON);
        assert_eq!(attr.access_type.as_deref(), Some("PUBLIC"));
        assert_eq!(
            attr.availability.as_deref(),
            Some(&["STREAM".to_string(), "DJ".to_string()][..])
        );
        assert_eq!(
            attr.media_tags.as_deref(),
            Some(&["HIRES_LOSSLESS".to_string(), "LOSSLESS".to_string()][..])
        );
        assert_eq!(attr.r#type, "SINGLE");
    }

    /// Regression: `GET /artists/{id}/relationships/albums?include=albums`
    /// returned `release_date: null` on the add-album page because the included
    /// album attributes deserialized with snake_case field names against Tidal's
    /// camelCase payload. This test feeds the exact shape `get_artist_albums`
    /// deserializes and asserts the date survives the round trip.
    #[test]
    fn deserializes_artist_albums_relationship_document() {
        let json = r#"
            {
              "data": [
                {"id": "546629982", "type": "albums"}
              ],
              "included": [
                {
                  "id": "546629982",
                  "type": "albums",
                  "attributes": {
                    "title": "Michelle (Take 1)",
                    "barcodeId": "00881061189336",
                    "numberOfVolumes": 1,
                    "numberOfItems": 1,
                    "duration": "PT3M17S",
                    "explicit": false,
                    "releaseDate": "2026-07-29",
                    "popularity": 0.7160302460678571,
                    "accessType": "PUBLIC",
                    "availability": ["STREAM", "DJ"],
                    "mediaTags": ["HIRES_LOSSLESS", "LOSSLESS"],
                    "type": "SINGLE"
                  }
                }
              ],
              "links": null
            }
        "#;

        let doc: ArtistAlbumsRelationshipDocument = serde_json::from_str(json).unwrap();

        assert_eq!(doc.included.len(), 1);
        let inc = &doc.included[0];
        assert_eq!(inc.id, "546629982");
        assert_eq!(inc.attributes.release_date.as_deref(), Some("2026-07-29"));
        assert_eq!(inc.attributes.barcode_id.as_deref(), Some("00881061189336"));
        assert_eq!(inc.attributes.number_of_volumes, Some(1));
        assert_eq!(inc.attributes.number_of_items, Some(1));
        assert_eq!(inc.attributes.access_type.as_deref(), Some("PUBLIC"));
        assert_eq!(
            inc.attributes.media_tags.as_deref().map(|v| v.len()),
            Some(2)
        );
        assert!(doc.links.is_none());
    }

    /// Regression guard: if `rename_all` is removed or reverted to `rename`,
    /// camelCase fields must fail to deserialize (they would silently `None`
    /// under the old bug). This test asserts the opposite direction, that a
    /// payload missing snake_case aliases still deserializes, proving the
    /// camelCase mapping is active rather than field names happening to match.
    #[test]
    fn does_not_accept_snake_case_field_names() {
        let json = r#"
            {
              "title": "Michelle (Take 1)",
              "release_date": "2026-07-29",
              "popularity": 0.0,
              "duration": "PT0S",
              "explicit": false,
              "type": "SINGLE"
            }
        "#;

        let attr: AlbumSearchIncludedAttributes = serde_json::from_str(json).unwrap();

        // Under `rename_all = "camelCase"`, `release_date` in the JSON does
        // not map to the `release_date` Rust field. The field stays `None`.
        // This is the expected behavior and documents that the struct relies
        // on camelCase input from Tidal.
        assert_eq!(attr.title, "Michelle (Take 1)");
        assert_eq!(attr.release_date, None);
    }
}
