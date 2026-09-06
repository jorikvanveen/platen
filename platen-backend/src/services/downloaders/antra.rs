use content_disposition::parse_content_disposition;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    fs::{self, File},
    io::{self, AsyncWriteExt},
    process::Command,
    time::{sleep, timeout},
};

use color_eyre::Report;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{entity::album, services::downloaders::Downloader};

static BASE_URL: &str = "https://antra.hoshi.cfd/api";
const JOB_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DOWNLOAD_PROGRESS_PERCENT_INTERVAL: u64 = 5;

struct DownloadProgress {
    total_bytes: Option<u64>,
    downloaded_bytes: u64,
    last_reported_percentage: u64,
}

impl DownloadProgress {
    fn new(total_bytes: Option<u64>) -> Self {
        Self {
            total_bytes: total_bytes.filter(|total| *total > 0),
            downloaded_bytes: 0,
            last_reported_percentage: 0,
        }
    }

    fn advance(&mut self, chunk_size: usize) -> bool {
        self.downloaded_bytes = self.downloaded_bytes.saturating_add(chunk_size as u64);

        let Some(total_bytes) = self.total_bytes else {
            return false;
        };
        let percentage = self.percentage().unwrap_or_default();
        if percentage < self.last_reported_percentage + DOWNLOAD_PROGRESS_PERCENT_INTERVAL
            && self.downloaded_bytes < total_bytes
        {
            return false;
        }
        self.last_reported_percentage = percentage;

        true
    }

    fn percentage(&self) -> Option<u64> {
        self.total_bytes.map(|total_bytes| {
            self.downloaded_bytes
                .saturating_mul(100)
                .checked_div(total_bytes)
                .unwrap_or(0)
                .min(100)
        })
    }
}

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
    pub fn new(config: &crate::config::Config) -> Self {
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

        Ok(resp.json().await?)
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
        let download_path = tmp.join(&filename);
        let mut file = File::create(&download_path).await?;
        let mut progress = DownloadProgress::new(resp.content_length());

        tracing::info!(
            job_id,
            filename,
            total_bytes = progress.total_bytes,
            "Starting Antra download"
        );
        loop {
            let chunk = match resp.chunk().await? {
                Some(c) => c,
                None => break,
            };

            file.write_all(&chunk).await?;
            if progress.advance(chunk.len()) {
                tracing::info!(
                    job_id,
                    filename,
                    downloaded_bytes = progress.downloaded_bytes,
                    total_bytes = progress.total_bytes,
                    percentage = progress.percentage(),
                    "Antra download progress"
                );
            }
        }
        tracing::info!(
            job_id,
            filename,
            downloaded_bytes = progress.downloaded_bytes,
            total_bytes = progress.total_bytes,
            "Antra download finished"
        );
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

// Archive folder names are not reliable artist names, and media servers read them as
// such. The destination must therefore come from catalog metadata. Tests inject the
// extraction parent so they can verify cleanup in a controlled workspace.
async fn place_archive(
    archive: &Path,
    destination: &Path,
    extraction_parent: &Path,
) -> Result<(), AntraError> {
    let extraction_dir = tempfile::Builder::new()
        .prefix("platen-extract-")
        .tempdir_in(extraction_parent)?;

    let placement = extract_and_place(extraction_dir.path(), archive, destination).await;

    // TempDir's Drop cleanup is blocking I/O, so the directory is removed
    // through tokio's fs instead.
    let _ = fs::remove_dir_all(extraction_dir.path()).await;

    match placement {
        Ok(()) => {
            fs::remove_file(archive).await?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(archive).await;
            Err(error)
        }
    }
}

async fn extract_and_place(
    extraction_root: &Path,
    archive: &Path,
    destination: &Path,
) -> Result<(), AntraError> {
    let exit_status = Command::new("unzip")
        .arg(archive)
        .arg("-d")
        .arg(extraction_root)
        .spawn()?
        .wait()
        .await?;
    if !exit_status.success() {
        return Err(AntraError::UnzipFailed);
    }

    let album_directory = find_album_directory(extraction_root).await?;
    copy_files_flat(&album_directory, destination).await
}

async fn find_album_directory(extraction_root: &Path) -> Result<PathBuf, AntraError> {
    let artist_directory = single_subdirectory(extraction_root).await?;
    let album_directory = single_subdirectory(&artist_directory).await?;

    // Disc subfolders would be silently dropped by the flat copy, so their
    // presence fails the placement instead.
    match directory_shape(&album_directory).await? {
        (None, true) => Ok(album_directory),
        (None, false) => Err(AntraError::EmptyArchive),
        (Some(_), _) => Err(AntraError::UnexpectedArchiveShape),
    }
}

// Reject extra files or directories because flattening them could drop tracks or
// make the album directory ambiguous.
async fn single_subdirectory(directory: &Path) -> Result<PathBuf, AntraError> {
    match directory_shape(directory).await? {
        (Some(subdirectory), false) => Ok(subdirectory),
        (Some(_), true) => Err(AntraError::UnexpectedArchiveShape),
        (None, true) => Err(AntraError::NoAlbumDirectory),
        (None, false) => Err(AntraError::EmptyArchive),
    }
}

async fn directory_shape(directory: &Path) -> Result<(Option<PathBuf>, bool), AntraError> {
    let mut entries = fs::read_dir(directory).await?;
    let mut subdirectory = None;
    let mut holds_file = false;

    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            if subdirectory.is_some() {
                return Err(AntraError::UnexpectedArchiveShape);
            }
            subdirectory = Some(entry.path());
        } else {
            holds_file = true;
        }
    }

    Ok((subdirectory, holds_file))
}

async fn copy_files_flat(album_directory: &Path, destination: &Path) -> Result<(), AntraError> {
    fs::create_dir_all(destination).await?;

    let mut entries = fs::read_dir(album_directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        let destination_path = destination.join(entry.file_name());
        if destination_path.exists() {
            continue;
        }
        fs::copy(entry.path(), &destination_path).await?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct JobStatusResponse {
    status: String,
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
pub enum AntraError {
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

    #[error("Antra returned a file type that does not match the album type")]
    UnexpectedDownloadType,

    #[error("Antra job failed with status: {0}")]
    JobFailed(String),

    #[error("Antra job did not finish within 10 minutes")]
    JobTimedOut,

    #[error("Could not unzip the downloaded archive")]
    UnzipFailed,

    #[error("The downloaded archive contains no files")]
    EmptyArchive,

    #[error(
        "The downloaded archive has no album directory: a folder above the tracks holds only files"
    )]
    NoAlbumDirectory,

    #[error("The downloaded archive does not have the expected shape of one flat album directory")]
    UnexpectedArchiveShape,
}
#[async_trait::async_trait]
impl Downloader for Antra {
    async fn download_album(
        &self,
        album: &album::Model,
        destination: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Downloading album: {}", album.title);
        let url = format!("https://tidal.com/browse/album/{}", album.id);

        let ResolveResponse { track_count } = self.resolve(&url).await?;
        let CreateJobResponse { job_id } = self.create_job(&url, track_count).await?;

        let poll_result = timeout(JOB_TIMEOUT, async {
            loop {
                sleep(Duration::from_secs(5)).await;
                let JobStatusResponse { status: job_status } = self.job_status(&job_id).await?;
                tracing::info!("Job status: {job_status}");
                if job_status == "complete" {
                    return Ok(());
                }
                if matches!(
                    job_status.to_ascii_lowercase().as_str(),
                    "failed" | "error" | "cancelled" | "canceled"
                ) {
                    return Err(AntraError::JobFailed(job_status));
                }
            }
        })
        .await
        .map_err(|_| AntraError::JobTimedOut)?;
        poll_result?;

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
            return Err(Box::new(AntraError::UnexpectedDownloadType));
        }

        if is_single {
            Self::move_single_to_destination(download_path, destination).await?;
            return Ok(());
        }
        let extraction_parent = std::env::temp_dir();
        place_archive(&download_path, destination, &extraction_parent).await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/services/downloaders/antra.rs"]
mod tests;
