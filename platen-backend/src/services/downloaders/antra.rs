use base64::prelude::*;
use content_disposition::parse_content_disposition;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    fs::{self, File},
    io::{self, AsyncWriteExt},
    process::Command,
    sync::Mutex,
    time::sleep,
};

use chrono::Utc;
use color_eyre::Report;
use reqwest::{RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    Config,
    services::downloaders::{
        Downloader,
        antra::tidal_response::{
            AlbumSearchData, AlbumSearchIncludedAttributes, AlbumSearchRelationshipsAlbumsData,
        },
    },
};

static BASE_URL: &'static str = "https://antra.hoshi.cfd/api";
static TIDAL_BASE_URL: &'static str = "https://openapi.tidal.com/v2";

#[derive(Clone)]
pub struct Antra {
    client: reqwest::Client,
    username: String,
    password: String,
    pub tidal: Arc<Mutex<TidalIndexer>>,
}

#[derive(Serialize)]
struct LoginRequestBody {
    username: String,
    password: String,
}

impl Antra {
    pub fn new(config: &Config) -> Self {
        Self {
            client: reqwest::ClientBuilder::new()
                .cookie_store(true)
                .build()
                .unwrap(),
            username: config.antra_username.clone(),
            password: config.antra_password.clone(),
            tidal: Arc::new(Mutex::new(TidalIndexer {
                client: reqwest::ClientBuilder::new().build().unwrap(),
                token: None,
                expires_at: Default::default(),
                client_id: config.tidal_client_id.clone(),
                client_secret: config.tidal_client_secret.clone(),
            })),
        }
    }

    pub async fn login(&self) -> color_eyre::Result<()> {
        let resp = self
            .client
            .post(format!("{BASE_URL}/auth/login"))
            .json(&LoginRequestBody {
                username: self.username.clone(),
                password: self.password.clone(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            tracing::error!("Antra: {}: {}", resp.status(), resp.text().await?);
            return Err(Report::msg("Failed to log in"));
        }

        tracing::info!("Successfully logged in to antra");

        self.tidal.lock().await.ensure_token().await?;

        return Ok(());
    }

    pub async fn resolve(&self, url: &str) -> Result<ResolveResponse, AntraError> {
        let resp = self
            .client
            .post(format!("{BASE_URL}/resolve"))
            .json(&ResolveRequestBody {
                format: "lossless-16".into(),
                url: url.into(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            tracing::error!("Antra resolve: {}: {}", resp.status(), resp.text().await?);
            return Err(AntraError::CantResolve);
        }

        Ok(resp.json().await?)
    }

    pub async fn create_job(
        &self,
        url: &str,
        track_count: usize,
    ) -> Result<CreateJobResponse, AntraError> {
        let resp = self
            .client
            .post(format!("{BASE_URL}/jobs"))
            .json(&CreateJobRequestBody {
                end_index: track_count,
                format: "lossless-16".into(),
                start_index: 0,
                url: url.into(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            tracing::error!(
                "Antra create job: {}: {}",
                resp.status(),
                resp.text().await?
            );
            return Err(AntraError::CantCreateJob);
        };

        Ok(resp.json().await?)
    }

    pub async fn job_status(&self, job_id: &str) -> Result<JobStatusResponse, AntraError> {
        let resp = self
            .client
            .get(format!("{BASE_URL}/jobs/{job_id}/status"))
            .send()
            .await?;

        if !resp.status().is_success() {
            tracing::error!(
                "Antra job status: {}: {}",
                resp.status(),
                resp.text().await?
            );
            return Err(AntraError::CantGetStatus);
        }

        let text = dbg!(resp.text().await?);
        Ok(serde_json::from_str(&text).unwrap())
    }

    pub async fn job_download(&self, job_id: &str) -> Result<PathBuf, AntraError> {
        let mut resp = self
            .client
            .get(format!("{BASE_URL}/jobs/{job_id}/download"))
            .send()
            .await?;

        if !resp.status().is_success() {
            tracing::error!("Antra download: {}: {}", resp.status(), resp.text().await?);
            return Err(AntraError::DownloadFailed);
        }

        let content_disposition = match resp
            .headers()
            .get("Content-Disposition")
            .map(|h| h.to_str().map(|h| parse_content_disposition(h)))
        {
            Some(Ok(c)) => c,
            None | Some(Err(_)) => {
                tracing::error!("Failed to parse content-disposition header");
                return Err(AntraError::DownloadFailed);
            }
        };

        let filename = match content_disposition.filename_full() {
            Some(f) => f,
            None => {
                tracing::error!("Content-disposion did not have filename");
                return Err(AntraError::DownloadFailed);
            }
        };

        let tmp = std::env::temp_dir();
        fs::create_dir_all(&tmp).await?;
        let zip_path = tmp.join(filename);
        let mut file = File::create(&zip_path).await?;

        tracing::info!("Starting download");
        loop {
            let chunk = match resp.chunk().await? {
                Some(c) => c,
                None => break,
            };

            file.write_all(&chunk).await?;
        }
        tracing::info!("Download finished");
        Ok(zip_path.into())
    }
}

#[derive(Debug, Deserialize)]
pub struct JobStatusResponse {
    status: String, // complete, zipping, downloading
    done: usize,
    failed: usize,
    total: usize,
}

#[derive(Debug, Serialize)]
struct CreateJobRequestBody {
    end_index: usize,
    format: String,
    start_index: usize,
    url: String,
}

#[derive(Debug, Deserialize)]
struct CreateJobResponse {
    job_id: String,
    ws_token: String,
}

#[derive(Debug, Serialize)]
struct ResolveRequestBody {
    format: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ResolveResponse {
    track_count: usize, // There is much more but we dont need all of that to start the download
}

#[derive(Error, Debug)]
pub enum AntraError {
    #[error("Tidal error: {0}")]
    Tidal(#[from] TidalError),

    #[error("The requested release could not be found")]
    NotFound,

    #[error("Error sending request: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Could not resolve the album URL")]
    CantResolve,

    #[error("Failed to create the Antra job")]
    CantCreateJob,

    #[error("Failed to receive job status")]
    CantGetStatus,

    #[error("I/O Error: {0}")]
    IoError(#[from] io::Error),

    #[error("Failed to download job")]
    DownloadFailed,

    #[error("Failed to unzip download")]
    UnzipFailed,
}

impl Downloader for Antra {
    type Error = AntraError;

    async fn download_release_group(
        &self,
        artist: &str,
        release_title: &str,
        destination: &std::path::Path,
    ) -> Result<(), Self::Error> {
        tracing::info!("Downloading release group: {}", &release_title);
        let release_query = format!("{artist} {}", release_title);
        let albums = { self.tidal.lock().await.find_album(&release_query).await? };

        let album = albums.first().ok_or(AntraError::NotFound)?;

        let url = format!("https://tidal.com/browse/album/{}", album.id);

        let ResolveResponse { track_count, .. } = self.resolve(&url).await?;
        let CreateJobResponse { job_id, .. } = self.create_job(&url, track_count).await?;

        loop {
            sleep(Duration::from_millis(5000)).await;
            let JobStatusResponse {
                status: job_status, ..
            } = self.job_status(&job_id).await?;
            tracing::info!("Job status: {job_status}");
            if job_status == "complete" {
                break;
            }
        }

        let zip_path = self.job_download(&job_id).await?;

        tracing::info!("Unzipping");
        let exit_status = Command::new("unzip")
            .arg("-n")
            .arg(zip_path)
            .arg("-d")
            .arg(destination)
            .spawn()?
            .wait()
            .await?;
        if !exit_status.success() {
            return Err(AntraError::UnzipFailed);
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum TidalError {
    #[error("Tidal API sent back an unexpected response")]
    UnexpectedResponse,

    #[error("Error sending request to tidal API: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Failed to authenticate with tidal API: {0}: {1}")]
    AuthenticationFailed(StatusCode, String),
}

pub struct TidalIndexer {
    client: reqwest::Client,
    token: Option<String>,
    expires_at: chrono::DateTime<Utc>,
    client_id: String,
    client_secret: String,
}

impl TidalIndexer {
    async fn send_with_retry(
        &self,
        builder: RequestBuilder,
    ) -> Result<reqwest::Response, TidalError> {
        let mut retries = 0;
        const MAX_RETRIES: u32 = 5;

        loop {
            // We're not sending streams so this shouldnt fail
            let req = builder.try_clone().ok_or(TidalError::UnexpectedResponse)?;

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

                    tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                    retries += 1;
                }
                Ok(res) => return Ok(res),
                Err(_) if retries < MAX_RETRIES => {
                    tokio::time::sleep(Duration::from_secs(1 << retries)).await;
                    retries += 1;
                }
                Err(e) => return Err(TidalError::Reqwest(e)),
            }
        }
    }

    async fn get_oauth_token(&mut self) -> Result<tidal_response::OauthToken, TidalError> {
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

        self.token = Some(response.access_token.clone());
        self.expires_at = chrono::Utc::now() + Duration::from_secs(response.expires_in - 60);

        tracing::info!("Authenticated with tidal");

        Ok(response)
    }

    async fn ensure_token(&mut self) -> Result<String, TidalError> {
        let now = chrono::Utc::now();

        if self.token.is_some() && self.expires_at < now {
            return Ok(self.token.clone().unwrap());
        }

        let tidal_response::OauthToken { access_token, .. } = self.get_oauth_token().await?;
        Ok(access_token)
    }

    async fn authenticate(
        &mut self,
        request: RequestBuilder,
    ) -> Result<RequestBuilder, TidalError> {
        let token = self.ensure_token().await?;
        Ok(request.bearer_auth(token))
    }

    pub async fn find_album(
        &mut self,
        query: &str,
    ) -> Result<Vec<ResolvedTidalSearchedAlbum>, TidalError> {
        let release_query = urlencoding::encode(&query);
        let url =
            format!("{TIDAL_BASE_URL}/searchResults?filter[query]={release_query}&include=albums");

        let resp = self
            .authenticate(self.client.get(url))
            .await?
            .send()
            .await?;
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
                    .and_then(|found_include| Ok((relationship, &found_include.attributes).into()))
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct ResolvedTidalSearchedAlbum {
    id: String,
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

impl Into<ResolvedTidalSearchedAlbum>
    for (
        &AlbumSearchRelationshipsAlbumsData,
        &AlbumSearchIncludedAttributes,
    )
{
    fn into(self) -> ResolvedTidalSearchedAlbum {
        let (data, attr) = self;
        ResolvedTidalSearchedAlbum {
            id: data.id.clone(),
            title: attr.title.clone(),
            barcode_id: attr.barcode_id.clone(),
            number_of_volumes: attr.number_of_volumes.clone(),
            number_of_items: attr.number_of_items.clone(),
            duration: attr.duration.clone(),
            explicit: attr.explicit.clone(),
            release_date: attr.release_date.clone(),
            popularity: attr.popularity.clone(),
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
