use std::path::Path;

use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, Database, EntityTrait, Set};
use thiserror::Error;

use super::{DownloadError, credited_artists, download_with};
use crate::{
    entity::{album, album_artist, artist},
    services::downloaders::Downloader,
};

#[derive(Debug, Error)]
#[error("test download failed")]
struct TestDownloadError;

struct TestDownloader {
    result: Result<(), TestDownloadError>,
}

#[async_trait::async_trait]
impl Downloader for TestDownloader {
    async fn download_album(
        &self,
        _album: &album::Model,
        destination: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match &self.result {
            Ok(()) => {
                tokio::fs::create_dir_all(destination).await?;
                tokio::fs::write(destination.join("1-01 track.flac"), "audio").await?;
                Ok(())
            }
            Err(_) => Err(Box::new(TestDownloadError)),
        }
    }
}

async fn test_database() -> sea_orm::DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    migration::Migrator::up(&db, None).await.unwrap();
    db
}

async fn insert_test_album(
    db: &sea_orm::DatabaseConnection,
    relative_path: Option<&str>,
) -> album::Model {
    let mut new_album = album::ActiveModel {
        id: Set("album-1".into()),
        title: Set("Test album".into()),
        album_type: Set(Some("SINGLE".into())),
        release_year: Set(2026),
        release_month: Set(None),
        release_day: Set(None),
        ..Default::default()
    };
    if let Some(relative_path) = relative_path {
        new_album.relative_path = Set(Some(relative_path.to_owned()));
    }
    let album = new_album.insert(db).await.unwrap();
    artist::ActiveModel {
        id: Set("artist-1".into()),
        name: Set("Test artist".into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
    album_artist::ActiveModel {
        album_id: Set(album.id.clone()),
        artist_id: Set("artist-1".into()),
        position: Set(0),
    }
    .insert(db)
    .await
    .unwrap();
    album
}

#[tokio::test]
async fn album_dto_orders_credits_primary_first() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    migration::Migrator::up(&db, None).await.unwrap();

    let album = album::ActiveModel {
        id: Set("album-1".into()),
        title: Set("Shared Credit".into()),
        album_type: Set(Some("ALBUM".into())),
        release_year: Set(2026),
        release_month: Set(Some(8)),
        release_day: Set(Some(29)),
        relative_path: Set(Some("Primary/Shared Credit (2026)".to_owned())),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    for (id, name) in [("primary", "Primary"), ("featured", "Featured")] {
        artist::ActiveModel {
            id: Set(id.into()),
            name: Set(name.into()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
    }
    for (artist_id, position) in [("featured", 1), ("primary", 0)] {
        album_artist::ActiveModel {
            album_id: Set(album.id.clone()),
            artist_id: Set(artist_id.into()),
            position: Set(position),
        }
        .insert(&db)
        .await
        .unwrap();
    }

    let artists = credited_artists(&db, &album.id).await.unwrap();
    let dto = super::album_dto(album, artists);

    assert_eq!(
        dto.artists
            .iter()
            .map(|artist| artist.id.as_str())
            .collect::<Vec<_>>(),
        ["primary", "featured"]
    );
    assert_eq!(
        dto.relative_path.as_deref(),
        Some("Primary/Shared Credit (2026)")
    );
    let serialized = serde_json::to_value(&dto).unwrap();
    assert!(serialized.get("relative_path").is_some());
    assert!(serialized.get("downloaded").is_none());
}

#[tokio::test]
async fn new_albums_default_to_no_location() {
    let db = test_database().await;
    let album = insert_test_album(&db, None).await;

    assert!(album.relative_path.is_none());
}

#[tokio::test]
async fn successful_download_is_saved() {
    let db = test_database().await;
    insert_test_album(&db, None).await;
    let music = tempfile::tempdir().unwrap();
    let downloader = TestDownloader { result: Ok(()) };

    download_with(&db, music.path().to_str().unwrap(), &downloader, "album-1")
        .await
        .unwrap();

    let album = album::Entity::find_by_id("album-1")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        album.relative_path.as_deref(),
        Some("Test artist/Test album (2026)")
    );
}

#[tokio::test]
async fn successful_download_publishes_through_staging() {
    let db = test_database().await;
    insert_test_album(&db, None).await;
    let music = tempfile::tempdir().unwrap();
    let downloader = TestDownloader { result: Ok(()) };

    download_with(&db, music.path().to_str().unwrap(), &downloader, "album-1")
        .await
        .unwrap();

    let final_dir = music.path().join("Test artist/Test album (2026)");
    assert!(final_dir.join("1-01 track.flac").exists());
    // Nothing is left behind in staging, and the staging root is empty.
    let staging = music.path().join(".platen-staging");
    let mut entries = tokio::fs::read_dir(&staging).await.unwrap();
    assert!(entries.next_entry().await.unwrap().is_none());
}

#[tokio::test]
async fn failed_download_is_not_saved() {
    let db = test_database().await;
    insert_test_album(&db, None).await;
    let music = tempfile::tempdir().unwrap();
    let downloader = TestDownloader {
        result: Err(TestDownloadError),
    };

    assert_eq!(
        download_with(&db, music.path().to_str().unwrap(), &downloader, "album-1").await,
        Err(DownloadError::Transfer)
    );

    let album = album::Entity::find_by_id("album-1")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(album.relative_path.is_none());
}

#[tokio::test]
async fn failed_download_leaves_no_partial_final_directory_and_cleans_staging() {
    let db = test_database().await;
    insert_test_album(&db, None).await;
    let music = tempfile::tempdir().unwrap();
    let downloader = TestDownloader {
        result: Err(TestDownloadError),
    };

    download_with(&db, music.path().to_str().unwrap(), &downloader, "album-1")
        .await
        .unwrap_err();

    assert!(!music.path().join("Test artist").exists());
    let staging = music.path().join(".platen-staging");
    let mut entries = tokio::fs::read_dir(&staging).await.unwrap();
    assert!(entries.next_entry().await.unwrap().is_none());
}

#[tokio::test]
async fn existing_final_destination_conflicts_even_when_empty() {
    let db = test_database().await;
    insert_test_album(&db, None).await;
    let music = tempfile::tempdir().unwrap();
    tokio::fs::create_dir_all(music.path().join("Test artist/Test album (2026)"))
        .await
        .unwrap();
    let downloader = TestDownloader { result: Ok(()) };

    assert_eq!(
        download_with(&db, music.path().to_str().unwrap(), &downloader, "album-1").await,
        Err(DownloadError::DestinationExists)
    );

    let album = album::Entity::find_by_id("album-1")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(album.relative_path.is_none());
    // The pre-existing directory is untouched.
    assert!(music.path().join("Test artist/Test album (2026)").exists());
}

#[tokio::test]
async fn leftover_staging_directories_do_not_block_downloads() {
    let db = test_database().await;
    insert_test_album(&db, None).await;
    let music = tempfile::tempdir().unwrap();
    let staging = music.path().join(".platen-staging");
    tokio::fs::create_dir_all(staging.join("album-1-leftover"))
        .await
        .unwrap();
    tokio::fs::write(
        staging.join("album-1-leftover/1-01 track.flac"),
        "stale audio",
    )
    .await
    .unwrap();
    let downloader = TestDownloader { result: Ok(()) };

    download_with(&db, music.path().to_str().unwrap(), &downloader, "album-1")
        .await
        .unwrap();

    let album = album::Entity::find_by_id("album-1")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(album.relative_path.is_some());
}

#[tokio::test]
async fn downloaded_album_conflicts_without_starting_another_download() {
    let db = test_database().await;
    insert_test_album(&db, Some("Test artist/Test album (2026)")).await;
    let music = tempfile::tempdir().unwrap();
    let downloader = TestDownloader { result: Ok(()) };

    assert_eq!(
        download_with(&db, music.path().to_str().unwrap(), &downloader, "album-1").await,
        Err(DownloadError::AlreadyDownloaded)
    );
}

#[tokio::test]
async fn missing_primary_artist_fails_download() {
    let db = test_database().await;
    let album = insert_test_album(&db, None).await;
    album_artist::Entity::delete_by_id((album.id.clone(), "artist-1".to_owned()))
        .exec(&db)
        .await
        .unwrap();
    artist::ActiveModel {
        id: Set("artist-2".into()),
        name: Set("Secondary artist".into()),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    album_artist::ActiveModel {
        album_id: Set(album.id),
        artist_id: Set("artist-2".into()),
        position: Set(1),
    }
    .insert(&db)
    .await
    .unwrap();

    let downloader = TestDownloader { result: Ok(()) };
    let music = tempfile::tempdir().unwrap();

    assert_eq!(
        download_with(&db, music.path().to_str().unwrap(), &downloader, "album-1").await,
        Err(DownloadError::Catalog)
    );
}
