use super::*;
use std::process::Command as SyncCommand;

#[test]
fn reports_download_progress_every_five_percent() {
    let mut progress = DownloadProgress::new(Some(1_000));

    assert!(!progress.advance(49));
    assert!(progress.advance(1));
    assert_eq!(progress.percentage(), Some(5));
    assert!(!progress.advance(49));
    assert!(progress.advance(1));
    assert_eq!(progress.percentage(), Some(10));
}

#[test]
fn reports_completion_when_the_last_chunk_is_smaller_than_the_interval() {
    let mut progress = DownloadProgress::new(Some(1_000));

    assert!(progress.advance(960));
    assert!(progress.advance(40));
    assert_eq!(progress.percentage(), Some(100));
}

#[test]
fn does_not_report_progress_when_content_length_is_unknown() {
    let mut progress = DownloadProgress::new(None);

    assert!(!progress.advance(5 * 1024 * 1024));
    assert_eq!(progress.percentage(), None);
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
