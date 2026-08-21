use base64::prelude::*;
use chrono::Utc;
use reqwest::{RequestBuilder, StatusCode};
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;

use tidal_response::{AlbumSearchIncludedAttributes, AlbumSearchRelationshipsAlbumsData};

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
}

#[derive(Debug)]
pub struct ResolvedTidalSearchedAlbum {
    pub(crate) id: String,
    title: String,
    barcode_id: Option<String>,
    number_of_volumes: Option<u32>,
    number_of_items: Option<u32>,
    duration: String, // ISO 8601
    explicit: bool,
    release_date: Option<String>, // 2022-04-20
    //copyright
    popularity: f64,
    access_type: Option<String>,
    availability: Vec<String>,
    media_tags: Option<Vec<String>>,
    //externalLinks,
    r#type: String,
    album_type: Option<String>, // EP, Single, Album
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
            album_type: attr.album_type.clone(),
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
    pub struct AlbumSearchData {
        pub id: String,
        pub r#type: String,
        //attributes; {query, trackingId}
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
    pub struct AlbumSearchRelationshipsAlbumsData {
        pub id: String,
        pub r#type: String, // always "albums"
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSearchIncluded {
        pub id: String,
        pub r#type: String,
        pub attributes: AlbumSearchIncludedAttributes,
    }
    #[derive(Debug, Deserialize)]
    #[serde(rename = "camelCase")]
    pub struct AlbumSearchIncludedAttributes {
        pub title: String,
        pub barcode_id: Option<String>,
        pub number_of_volumes: Option<u32>,
        pub number_of_items: Option<u32>,
        pub duration: String, // ISO 8601
        pub explicit: bool,
        pub release_date: Option<String>, // 2022-04-20
        //copyright
        pub popularity: f64,
        pub access_type: Option<String>,
        pub availability: Vec<String>,
        pub media_tags: Option<Vec<String>>,
        //externalLinks,
        pub r#type: String,
        pub album_type: Option<String>, // EP, Single, Album
    }
}
