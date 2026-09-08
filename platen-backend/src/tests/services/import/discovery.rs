use super::super::model::ScanSummary;
use super::*;

async fn discover(root: &Path) -> Vec<AlbumCandidate> {
    discover_album_candidates(root, |_| {}).await.candidates
}

#[tokio::test]
async fn discovery_reports_progress_without_job_state() {
    let music = tempfile::tempdir().unwrap();
    let album = music.path().join("Artist/Title (2024)");
    tokio::fs::create_dir_all(&album).await.unwrap();
    tokio::fs::write(album.join("track.flac"), b"audio")
        .await
        .unwrap();
    tokio::fs::create_dir_all(music.path().join("Artist/Empty"))
        .await
        .unwrap();
    let mut updates = Vec::new();
    let report = discover_album_candidates(music.path(), |progress| updates.push(progress)).await;
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(report.skipped_directories, 1);
    assert_eq!(
        updates.last(),
        Some(&DiscoveryProgress {
            album_directories_found: 1,
            skipped_directories: 1,
            filesystem_errors: 0
        })
    );
}

#[test]
fn discovery_summary_includes_root_failure_diagnostics() {
    let report = DiscoveryReport {
        diagnostics: vec![FilesystemDiagnostic {
            reason: "resolve_music_root",
            path: "missing".into(),
            os_error: std::io::ErrorKind::NotFound.into(),
        }],
        root_failed: true,
        ..Default::default()
    };
    assert_eq!(
        ScanSummary::from(&report),
        ScanSummary {
            filesystem_errors: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn discovers_supported_audio_recursively_and_parses_only_the_final_year() {
    let music = tempfile::tempdir().unwrap();
    let album = music.path().join("Artist/Title (Live) (2024)/Disc 1");
    tokio::fs::create_dir_all(&album).await.unwrap();
    tokio::fs::write(album.join("track.FLAC"), b"audio")
        .await
        .unwrap();

    let candidates = discover(music.path()).await;

    assert_eq!(
        candidates,
        vec![AlbumCandidate {
            primary_artist: "Artist".to_owned(),
            title: "Title (Live)".to_owned(),
            release_year: Some(2024),
            relative_path: "Artist/Title (Live) (2024)".to_owned(),
        }]
    );
}

#[tokio::test]
async fn accepts_every_supported_extension_case_insensitively() {
    let music = tempfile::tempdir().unwrap();
    for (index, extension) in [
        "flac", "MP3", "M4a", "aac", "OGG", "opus", "WAV", "aiff", "AIF", "alac",
    ]
    .into_iter()
    .enumerate()
    {
        let album = music.path().join(format!("Artist/Album {index}"));
        tokio::fs::create_dir_all(&album).await.unwrap();
        tokio::fs::write(album.join(format!("track.{extension}")), b"audio")
            .await
            .unwrap();
    }

    assert_eq!(discover(music.path()).await.len(), 10);
}

#[tokio::test]
async fn skips_empty_artwork_only_malformed_and_staging_directories() {
    let music = tempfile::tempdir().unwrap();
    for album in ["Empty", "Artwork", "(2024)"] {
        tokio::fs::create_dir_all(music.path().join("Artist").join(album))
            .await
            .unwrap();
    }
    tokio::fs::write(music.path().join("Artist/Artwork/cover.jpg"), b"image")
        .await
        .unwrap();
    let staging = music.path().join(STAGING_DIRECTORY).join("job");
    tokio::fs::create_dir_all(&staging).await.unwrap();
    tokio::fs::write(staging.join("track.flac"), b"audio")
        .await
        .unwrap();

    let report = discover_album_candidates(music.path(), |_| {}).await;

    assert!(report.candidates.is_empty());
    assert_eq!(report.skipped_directories, 3);
}

#[cfg(unix)]
#[tokio::test]
async fn resolves_a_symbolic_link_root_but_does_not_follow_descendants() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().unwrap();
    let real = parent.path().join("real");
    tokio::fs::create_dir_all(real.join("Artist/Real album"))
        .await
        .unwrap();
    tokio::fs::write(real.join("Artist/Real album/track.flac"), b"audio")
        .await
        .unwrap();
    let outside = parent.path().join("outside/Linked album");
    tokio::fs::create_dir_all(&outside).await.unwrap();
    tokio::fs::write(outside.join("track.mp3"), b"audio")
        .await
        .unwrap();
    symlink(&outside, real.join("Artist/Linked album")).unwrap();
    let root_link = parent.path().join("music");
    symlink(&real, &root_link).unwrap();

    let report = discover_album_candidates(&root_link, |_| {}).await;
    let candidates = &report.candidates;

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].title, "Real album");
    assert_eq!(report.skipped_directories, 1);
}

#[cfg(unix)]
#[tokio::test]
async fn skips_non_utf8_directories_without_panicking() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let music = tempfile::tempdir().unwrap();
    let invalid = OsString::from_vec(vec![b'A', 0xff]);
    tokio::fs::create_dir_all(music.path().join(invalid))
        .await
        .unwrap();

    let report = discover_album_candidates(music.path(), |_| {}).await;

    assert!(report.candidates.is_empty());
    assert_eq!(report.skipped_directories, 1);
}

#[tokio::test]
async fn a_missing_root_is_a_terminal_failure() {
    let root = tempfile::tempdir().unwrap().path().join("missing");

    let report = discover_album_candidates(&root, |_| {}).await;
    assert!(report.root_failed);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].path, root);
}
