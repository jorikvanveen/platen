use content_disposition::parse_content_disposition;
use std::{path::PathBuf, time::Duration};
use tokio::{
    fs::{self, File},
    io::{self, AsyncWriteExt},
    process::Command,
    time::sleep,
};

use color_eyre::Report;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{entity::album, services::downloaders::Downloader};

static BASE_URL: &str = "https://antra.hoshi.cfd/api";

#[derive(Clone)]
pub struct Antra {
    client: reqwest::Client,
    username: String,
    password: String,
}

#[derive(Serialize)]
struct LoginRequestBody {
    username: String,
    password: String,
}

impl Antra {
    pub fn new(config: &crate::Config) -> Self {
        Self {
            client: reqwest::ClientBuilder::new()
                .cookie_store(true)
                .build()
                .expect(
                    "reqwest client build only fails on TLS misconfiguration, which is static here",
                ),
            username: config.antra_username.clone(),
            password: config.antra_password.clone(),
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

        Ok(())
    }

    async fn resolve(&self, url: &str) -> Result<ResolveResponse, AntraError> {
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

    async fn create_job(
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

    async fn job_status(&self, job_id: &str) -> Result<JobStatusResponse, AntraError> {
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
        Ok(serde_json::from_str(&text)?)
    }

    async fn job_download(&self, job_id: &str) -> Result<PathBuf, AntraError> {
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
            .map(|h| h.to_str().map(parse_content_disposition))
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
                return Err(AntraError::UnexpectedDownloadType);
            }
        };

        let tmp = std::env::temp_dir();
        fs::create_dir_all(&tmp).await?;
        let download_path = tmp.join(filename);
        let mut file = File::create(&download_path).await?;

        tracing::info!("Starting download");
        loop {
            let chunk = match resp.chunk().await? {
                Some(c) => c,
                None => break,
            };

            file.write_all(&chunk).await?;
        }
        tracing::info!("Download finished");
        Ok(download_path)
    }

    async fn move_single_to_destination(
        download_path: PathBuf,
        destination: &std::path::Path,
    ) -> Result<(), AntraError> {
        let filename = download_path
            .file_name()
            .ok_or(AntraError::DownloadFailed)?;
        let destination_path = destination.join(filename);

        fs::create_dir_all(destination).await?;
        if destination_path.exists() {
            fs::remove_file(download_path).await?;
            return Ok(());
        }

        fs::copy(&download_path, &destination_path).await?;
        fs::remove_file(download_path).await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JobStatusResponse {
    status: String,
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
#[allow(dead_code)]
struct CreateJobResponse {
    pub job_id: String,
    pub ws_token: String,
}

#[derive(Debug, Serialize)]
struct ResolveRequestBody {
    format: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ResolveResponse {
    // The resolve endpoint returns more fields than this; track_count is the
    // only one needed to create a job.
    pub track_count: usize,
}

#[derive(Error, Debug)]
#[allow(unused)]
pub enum AntraError {
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

    #[error("Job status response was not valid JSON: {0}")]
    BadStatus(#[from] serde_json::Error),

    #[error("I/O Error: {0}")]
    IoError(#[from] io::Error),

    #[error("Failed to download job")]
    DownloadFailed,

    #[error("Antra returned a file type that does not match the album type")]
    UnexpectedDownloadType,

    #[error("Failed to unzip download")]
    UnzipFailed,
}

impl Downloader for Antra {
    type Error = AntraError;

    async fn download_album(
        &self,
        album: &album::Model,
        destination: &std::path::Path,
    ) -> Result<(), Self::Error> {
        tracing::info!("Downloading album: {}", album.title);
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

        let download_path = self.job_download(&job_id).await?;
        let is_single = album
            .album_type
            .as_deref()
            .is_some_and(|album_type| album_type.eq_ignore_ascii_case("SINGLE"));
        let is_album_or_ep = album.album_type.as_deref().is_some_and(|album_type| {
            album_type.eq_ignore_ascii_case("ALBUM") || album_type.eq_ignore_ascii_case("EP")
        });
        let extension = download_path.extension();
        let is_flac = extension.is_some_and(|extension| extension.eq_ignore_ascii_case("flac"));
        let is_zip = extension.is_some_and(|extension| extension.eq_ignore_ascii_case("zip"));

        if (!is_single && !is_album_or_ep) || (is_single && !is_flac) || (is_album_or_ep && !is_zip)
        {
            let _ = fs::remove_file(&download_path).await;
            return Err(AntraError::UnexpectedDownloadType);
        }

        if is_single {
            return Self::move_single_to_destination(download_path, destination).await;
        }

        tracing::info!("Unzipping");
        let exit_status = Command::new("unzip")
            .arg("-n")
            .arg(&download_path)
            .arg("-d")
            .arg(destination)
            .spawn()?
            .wait()
            .await?;
        if !exit_status.success() {
            let _ = fs::remove_file(&download_path).await;
            return Err(AntraError::UnzipFailed);
        }

        fs::remove_file(download_path).await?;
        Ok(())
    }
}
