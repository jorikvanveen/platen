use rustypipe::client::RustyPipe;
use rustypipe::model::MusicItem;
use rustypipe::model::MusicSearchResult;
use tokio::fs;
use tokio::io;
use tokio::process::Command;
use tokio::time::sleep;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;
use std::process::Stdio;
use std::time::Duration;
use tracing::info;

use crate::downloaders::Downloader;
use crate::downloaders::RateLimit;
use crate::musicbrainz::release::{Release, Track};

pub struct Youtube {
    rate_limit: RateLimit,
    rp: RustyPipe,
}

impl Youtube {
    pub fn new() -> Self {
        Self {
            rate_limit: RateLimit::new(1000),
            rp: RustyPipe::new(),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadError {
    #[error("Error querying youtube: {0}")]
    Rustypipe(#[from] rustypipe::error::Error),

    #[error("This release is not available on YouTube Music")]
    NotFound,

    #[error("IO error: {0}")]
    Fs(#[from] io::Error),

    #[error("yt-dlp failed: {0}")]
    YtDlp(String)
}

impl Downloader for Youtube {
    type Error = DownloadError;
    async fn download_release(
        &self,
        release: &Release,
        out_dir: &Path,
    ) -> Result<(), DownloadError> {
        //let title = &release.title;
        let artist_name = release
            .artist_credit
            .first()
            .map(|c| c.artist.name.clone())
            .ok_or_else(|| DownloadError::NotFound)?;
        info!("Searching youtube artist {artist_name}");

        self.rate_limit.wait().await;
        let mut artist_search_res = self.rp.query().music_search_artists(artist_name).await?;
        artist_search_res.items.extend(self.rp.query()).await?;
        let yt_artist = artist_search_res
            .items
            .items
            .first()
            .ok_or_else(|| DownloadError::NotFound)?;
        info!("Found youtube artist {}", yt_artist.id);

        self.rate_limit.wait().await;
        let artist_fetch_res = self.rp.query().music_artist(&yt_artist.id, true).await?;
        info!("Found albums: {:?}", artist_fetch_res.albums.iter().map(|a| a.name.clone()).collect::<Vec<_>>());
        let yt_album = artist_fetch_res
            .albums
            .iter()
            .find(|a| a.name.to_lowercase().trim() == release.title.to_lowercase().trim())
            .ok_or(DownloadError::NotFound)?;
            
        info!("Downloading album id: {}", yt_album.id);
        self.rate_limit.wait().await;
        let url = self.rp.query().resolve_string(&yt_album.id, true).await?.to_url();
        info!("Resolving to url: {}", url);

        self.rate_limit.wait().await;
        fs::create_dir_all(out_dir).await?;
        let mut retry_counter = 3;
        loop {
            let cmd = Command::new("yt-dlp")
                .arg(&url)
                .arg("-x")
                .arg("-P")
                .arg(out_dir)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            let output = cmd.wait_with_output().await?;
            if output.status.success() {
                break
            }
            let stderr = String::from_utf8(output.stderr).unwrap();
            if !stderr.contains("HTTP Error 403: Forbidden") && retry_counter != 0 {
                return Err(DownloadError::YtDlp(stderr))
            }
            info!("Got 403, retrying in 10 seconds");
            sleep(Duration::from_secs(10)).await;
            retry_counter -= 1;
         }
        Ok(())
    }
}
