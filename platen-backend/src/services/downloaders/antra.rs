use content_disposition::parse_content_disposition;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
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

        let text = resp.text().await?;
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

    #[error("Antra job failed with status: {0}")]
    JobFailed(String),

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
            if matches!(
                job_status.to_ascii_lowercase().as_str(),
                "failed" | "error" | "cancelled" | "canceled"
            ) {
                return Err(Box::new(AntraError::JobFailed(job_status)));
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
mod tests {
    use super::*;
    use std::process::Command as SyncCommand;

    // Real zips, because placement extracts with the real unzip binary.
    fn build_archive(working_dir: &Path, archive: &Path, entries: &[&str]) {
        let status = SyncCommand::new("zip")
            .arg("-q")
            .arg("-r")
            .arg(archive)
            .args(entries)
            .current_dir(working_dir)
            .status()
            .expect("the zip tool is available on this machine");
        assert!(status.success(), "could not build the test archive");
    }

    // Every entry is asserted to be a file, so an archive-derived folder
    // fails the test instead of hiding among the names.
    async fn flat_file_names(destination: &Path) -> Vec<String> {
        let mut names = Vec::new();
        let mut entries = fs::read_dir(destination).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            assert!(
                entry.file_type().await.unwrap().is_file(),
                "{} is not a file",
                entry.path().display()
            );
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        names
    }

    struct Download {
        workspace: tempfile::TempDir,
        archive: PathBuf,
        extraction_parent: PathBuf,
        destination: PathBuf,
    }

    impl Download {
        fn new() -> Self {
            let workspace = tempfile::tempdir().unwrap();
            let root = workspace.path().to_path_buf();
            Self {
                archive: root.join("download.zip"),
                extraction_parent: root.join("extraction"),
                destination: root.join("BLCKK").join("Duality (2024)"),
                workspace,
            }
        }

        fn root(&self) -> &Path {
            self.workspace.path()
        }

        async fn write_source_file(&self, relative: &str, contents: &str) {
            let path = self.root().join(relative);
            fs::create_dir_all(path.parent().unwrap()).await.unwrap();
            fs::write(path, contents).await.unwrap();
        }

        async fn build_archive_from(&self, entries: &[&str]) {
            fs::create_dir_all(&self.extraction_parent).await.unwrap();
            build_archive(self.root(), &self.archive, entries);
        }

        async fn place(&self) -> Result<(), AntraError> {
            place_archive(&self.archive, &self.destination, &self.extraction_parent).await
        }

        // The extraction parent must end up empty because the temporary
        // directory is removed on success and on failure alike.
        async fn assert_extraction_parent_empty(&self) {
            let mut entries = fs::read_dir(&self.extraction_parent).await.unwrap();
            assert!(
                entries.next_entry().await.unwrap().is_none(),
                "the extraction parent is not empty"
            );
        }
    }

    #[tokio::test]
    async fn copies_the_album_directorys_files_flat_into_the_destination() {
        let download = Download::new();
        let archive_folder = "BLCKK, ISSBROKIE/Duality (2024) [FLAC]";
        download
            .write_source_file(
                &format!("{archive_folder}/1-01 North Star.flac"),
                "disc 1 track",
            )
            .await;
        download
            .write_source_file(
                &format!("{archive_folder}/2-21 Light Years.flac"),
                "disc 2 track",
            )
            .await;
        download.build_archive_from(&["BLCKK, ISSBROKIE"]).await;

        download.place().await.unwrap();

        assert_eq!(
            flat_file_names(&download.destination).await,
            ["1-01 North Star.flac", "2-21 Light Years.flac"]
        );
        assert_eq!(
            fs::read_to_string(download.destination.join("2-21 Light Years.flac"))
                .await
                .unwrap(),
            "disc 2 track"
        );

        assert!(!download.archive.exists());
        download.assert_extraction_parent_empty().await;
    }

    #[tokio::test]
    async fn navigates_the_artist_album_shape() {
        let download = Download::new();
        download
            .write_source_file(
                "BLCKK, ISSBROKIE/Duality (2024) [FLAC]/1-01 North Star.flac",
                "disc 1 track",
            )
            .await;
        download.build_archive_from(&["BLCKK, ISSBROKIE"]).await;

        download.place().await.unwrap();

        assert_eq!(
            flat_file_names(&download.destination).await,
            ["1-01 North Star.flac"]
        );
    }

    #[tokio::test]
    async fn skips_files_that_already_exist_in_the_destination() {
        let download = Download::new();
        let archive_folder = "BLCKK, ISSBROKIE/Duality (2024) [FLAC]";
        download
            .write_source_file(
                &format!("{archive_folder}/1-01 North Star.flac"),
                "disc 1 track",
            )
            .await;
        download
            .write_source_file(
                &format!("{archive_folder}/2-21 Light Years.flac"),
                "disc 2 track",
            )
            .await;
        download.build_archive_from(&["BLCKK, ISSBROKIE"]).await;

        fs::create_dir_all(&download.destination).await.unwrap();
        fs::write(
            download.destination.join("1-01 North Star.flac"),
            "already there",
        )
        .await
        .unwrap();

        download.place().await.unwrap();

        assert_eq!(
            fs::read_to_string(download.destination.join("1-01 North Star.flac"))
                .await
                .unwrap(),
            "already there"
        );
        assert_eq!(
            fs::read_to_string(download.destination.join("2-21 Light Years.flac"))
                .await
                .unwrap(),
            "disc 2 track"
        );
    }

    #[tokio::test]
    async fn fails_when_the_archive_contains_no_files() {
        let download = Download::new();
        fs::create_dir_all(download.root().join("empty album directory"))
            .await
            .unwrap();
        download
            .build_archive_from(&["empty album directory"])
            .await;

        let error = download.place().await.unwrap_err();

        assert!(matches!(error, AntraError::EmptyArchive), "{error:?}");
        assert!(!download.destination.exists());
        download.assert_extraction_parent_empty().await;
        assert!(!download.archive.exists());
    }

    #[tokio::test]
    async fn fails_when_the_deepest_level_of_the_archive_is_a_file() {
        let download = Download::new();
        download
            .write_source_file("1-01 North Star.flac", "disc 1 track")
            .await;
        download.build_archive_from(&["1-01 North Star.flac"]).await;

        let error = download.place().await.unwrap_err();

        assert!(matches!(error, AntraError::NoAlbumDirectory), "{error:?}");
        assert!(!download.destination.exists());
        download.assert_extraction_parent_empty().await;
        assert!(!download.archive.exists());
    }

    #[tokio::test]
    async fn fails_when_the_artist_directory_holds_a_stray_file() {
        let download = Download::new();
        let archive_folder = "BLCKK, ISSBROKIE/Duality (2024) [FLAC]";
        download
            .write_source_file(
                &format!("{archive_folder}/1-01 North Star.flac"),
                "disc 1 track",
            )
            .await;
        download
            .write_source_file("BLCKK, ISSBROKIE/cover.jpg", "stray file")
            .await;
        download.build_archive_from(&["BLCKK, ISSBROKIE"]).await;

        let error = download.place().await.unwrap_err();

        assert!(
            matches!(error, AntraError::UnexpectedArchiveShape),
            "{error:?}"
        );
        assert!(!download.destination.exists());
        download.assert_extraction_parent_empty().await;
    }

    #[tokio::test]
    async fn fails_when_the_album_directory_holds_a_disc_subfolder() {
        let download = Download::new();
        let archive_folder = "BLCKK, ISSBROKIE/Duality (2024) [FLAC]";
        download
            .write_source_file(
                &format!("{archive_folder}/disc 1/1-01 North Star.flac"),
                "disc 1",
            )
            .await;
        download
            .write_source_file(
                &format!("{archive_folder}/disc 2/2-21 Light Years.flac"),
                "disc 2",
            )
            .await;
        download.build_archive_from(&["BLCKK, ISSBROKIE"]).await;

        let error = download.place().await.unwrap_err();

        assert!(
            matches!(error, AntraError::UnexpectedArchiveShape),
            "{error:?}"
        );
        assert!(!download.destination.exists());
        download.assert_extraction_parent_empty().await;
    }

    #[tokio::test]
    async fn fails_when_files_sit_outside_the_album_directory() {
        let download = Download::new();
        let archive_folder = "BLCKK, ISSBROKIE/Duality (2024) [FLAC]";
        download
            .write_source_file(
                &format!("{archive_folder}/1-01 North Star.flac"),
                "disc 1 track",
            )
            .await;
        download
            .write_source_file("cover.jpg", "not part of a flat album directory")
            .await;
        download
            .build_archive_from(&["BLCKK, ISSBROKIE", "cover.jpg"])
            .await;

        let error = download.place().await.unwrap_err();

        assert!(
            matches!(error, AntraError::UnexpectedArchiveShape),
            "{error:?}"
        );
        assert!(!download.destination.exists());
        download.assert_extraction_parent_empty().await;
    }
}
