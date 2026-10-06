use super::*;
use std::{
    process::Command as SyncCommand,
    sync::{Arc, Mutex},
};

use axum::{Router, body::Body, routing::get};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};

#[test]
fn album_job_requests_reject_empty_albums() {
    assert!(matches!(
        CreateJobRequestBody::for_album("https://tidal.com/browse/album/123", 0),
        Err(AntraError::EmptyAlbum)
    ));
}

#[test]
fn download_progress_reports_five_percent_increments() {
    let mut progress = DownloadProgress::new(Some(100));
    assert!(!progress.advance(4));
    assert!(progress.advance(1));
    assert!(!progress.advance(4));
    assert!(progress.advance(1));
    assert!(progress.advance(90));
    assert_eq!(progress.downloaded_bytes, 100);
    assert_eq!(progress.percentage(), Some(100));
}

#[tokio::test(start_paused = true)]
async fn download_progress_reports_bytes_without_a_content_length() {
    for total_bytes in [None, Some(0)] {
        let mut progress = DownloadProgress::new(total_bytes);
        assert!(!progress.advance(1024));
        tokio::time::advance(DOWNLOAD_PROGRESS_LOG_INTERVAL).await;
        assert!(progress.advance(1024));
        assert_eq!(progress.downloaded_bytes, 2048);
        assert_eq!(progress.percentage(), None);
        assert!(!progress.advance(1024));
    }
}

#[tokio::test(start_paused = true)]
async fn download_progress_reports_slow_transfers_before_five_percent() {
    let mut progress = DownloadProgress::new(Some(1000));
    assert!(!progress.advance(1));
    tokio::time::advance(DOWNLOAD_PROGRESS_LOG_INTERVAL).await;
    assert!(progress.advance(1));
    assert_eq!(progress.percentage(), Some(0));
}

async fn streamed_download_response() -> (
    reqwest::Response,
    mpsc::UnboundedSender<Result<&'static str, io::Error>>,
    JoinHandle<()>,
) {
    let (chunk_sender, chunk_receiver) = mpsc::unbounded_channel();
    let chunks = futures_util::stream::unfold(chunk_receiver, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (chunk, receiver))
    });
    let response_body = Arc::new(Mutex::new(Some(Body::from_stream(chunks))));
    let app = Router::new().route(
        "/download",
        get(move || {
            let body = response_body.lock().unwrap().take().unwrap();
            async move { axum::response::Response::new(body) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let download_url = format!("http://{}/download", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(download_url)
        .send();
    let response = timeout(Duration::from_secs(5), response)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.content_length(), None);
    (response, chunk_sender, server)
}

async fn wait_for_download_size(download_path: &Path, expected_bytes: u64) {
    let started_at = std::time::Instant::now();
    loop {
        if fs::metadata(download_path)
            .await
            .is_ok_and(|metadata| metadata.len() == expected_bytes)
        {
            tokio::task::yield_now().await;
            return;
        }
        assert!(
            started_at.elapsed() < Duration::from_secs(5),
            "download did not write {expected_bytes} bytes"
        );
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn saves_a_complete_download_without_a_content_length() {
    let workspace = tempfile::tempdir().unwrap();
    let download_path = workspace.path().join("download.zip");
    let (response, chunk_sender, server) = streamed_download_response().await;
    chunk_sender.send(Ok("first chunk")).unwrap();
    chunk_sender.send(Ok("second chunk")).unwrap();
    drop(chunk_sender);

    timeout(
        Duration::from_secs(5),
        Antra::save_download(response, "test-job", "download.zip", &download_path),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(
        fs::read(&download_path).await.unwrap(),
        b"first chunksecond chunk"
    );
    server.abort();
}

async fn assert_download_stalls_after_idle(initial_chunk: Option<&'static str>, send_more: bool) {
    let workspace = tempfile::tempdir().unwrap();
    let download_path = workspace.path().join("download.zip");
    let (response, chunk_sender, server) = streamed_download_response().await;
    let mut downloaded_bytes = 0;
    if let Some(chunk) = initial_chunk {
        downloaded_bytes += chunk.len() as u64;
        chunk_sender.send(Ok(chunk)).unwrap();
    }
    let task_download_path = download_path.clone();
    let download = tokio::spawn(async move {
        Antra::save_download(response, "test-job", "download.zip", &task_download_path).await
    });
    wait_for_download_size(&download_path, downloaded_bytes).await;

    // Socket and file I/O must finish before Tokio advances the paused clock.
    let clock_guard = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    tokio::time::advance(DOWNLOAD_IDLE_TIMEOUT - Duration::from_secs(1)).await;
    assert!(!download.is_finished());
    if send_more {
        chunk_sender.send(Ok("another chunk")).unwrap();
        downloaded_bytes += "another chunk".len() as u64;
        wait_for_download_size(&download_path, downloaded_bytes).await;
        tokio::time::advance(DOWNLOAD_IDLE_TIMEOUT - Duration::from_secs(1)).await;
        assert!(!download.is_finished());
    }
    // Tokio rounds deadlines to milliseconds; cross that boundary before resuming for file cleanup.
    tokio::time::advance(Duration::from_secs(1) + Duration::from_millis(1)).await;
    tokio::time::resume();
    let error = timeout(Duration::from_secs(5), download)
        .await
        .expect("idle timeout did not finish the download")
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(error, AntraError::DownloadStalled { downloaded_bytes: bytes } if bytes == downloaded_bytes),
        "{error:?}"
    );
    assert!(!download_path.exists());
    server.abort();
    clock_guard.abort();
}

#[tokio::test]
async fn times_out_when_the_download_body_never_starts() {
    assert_download_stalls_after_idle(None, false).await;
}

#[tokio::test]
async fn stalled_download_removes_the_partial_file() {
    assert_download_stalls_after_idle(Some("partial download"), false).await;
}

#[tokio::test]
async fn download_idle_timeout_resets_after_each_chunk() {
    assert_download_stalls_after_idle(Some("partial download"), true).await;
}

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
