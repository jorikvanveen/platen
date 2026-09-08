use super::*;
use crate::services::tidal::{ResolvedTidalSearchedAlbum, TidalArtist};
use sea_orm::{ActiveModelTrait, Database, Set};

async fn repository() -> ImportRepository {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    <migration::Migrator as migration::MigratorTrait>::up(&db, None)
        .await
        .unwrap();
    ImportRepository::new(db)
}

fn prepared(id: &str) -> PreparedAlbum {
    ResolvedTidalSearchedAlbum {
        id: id.into(),
        title: "Title".into(),
        release_date: Some("2024".into()),
        cover_url: None,
        popularity: 0.0,
        r#type: "ALBUM".into(),
        explicit: None,
        media_tags: None,
        artists: vec![TidalArtist {
            id: "artist".into(),
            name: "Artist".into(),
            profile_image_url: None,
        }],
    }
    .try_into()
    .unwrap()
}

async fn seed(repository: &ImportRepository, id: &str, path: Option<&str>) {
    album::ActiveModel {
        id: Set(id.into()),
        title: Set("Original metadata".into()),
        release_year: Set(1999),
        relative_path: Set(path.map(str::to_owned)),
        ..Default::default()
    }
    .insert(&repository.db)
    .await
    .unwrap();
}

#[tokio::test]
async fn stale_location_plans_never_overwrite_newer_locations() {
    for previous_path in [None, Some("Artist/Old")] {
        for target_path in [None, Some("Artist/Discovered")] {
            let repository = repository().await;
            seed(&repository, "album", Some("Artist/Downloaded")).await;
            let result = repository
                .apply_location(LocationUpdate {
                    album_id: "album".into(),
                    previous_path: previous_path.map(str::to_owned),
                    relative_path: target_path.map(str::to_owned),
                })
                .await;
            assert!(result.is_err());
            assert_eq!(
                repository.location("album").await.unwrap().as_deref(),
                Some("Artist/Downloaded")
            );
        }
    }
}

#[tokio::test]
async fn deleted_albums_do_not_count_as_successful_location_updates() {
    let repository = repository().await;
    assert!(
        repository
            .apply_location(LocationUpdate {
                album_id: "deleted".into(),
                previous_path: None,
                relative_path: Some("Artist/Title".into()),
            })
            .await
            .is_err()
    );
    assert!(repository.albums().await.unwrap().is_empty());
}

#[tokio::test]
async fn location_updates_recheck_occupancy_after_planning() {
    let repository = repository().await;
    seed(&repository, "album", Some("Artist/Old")).await;
    let update = LocationUpdate {
        album_id: "album".into(),
        previous_path: Some("Artist/Old".into()),
        relative_path: Some("Artist/New".into()),
    };
    seed(&repository, "occupant", Some("Artist/New")).await;
    assert!(repository.apply_location(update).await.is_err());
    assert_eq!(
        repository.location("album").await.unwrap().as_deref(),
        Some("Artist/Old")
    );
}

#[tokio::test]
async fn occupied_imports_create_neither_album_nor_credits() {
    let repository = repository().await;
    seed(&repository, "occupant", Some("Artist/Title")).await;
    assert_eq!(
        repository
            .import(prepared("new"), "Artist/Title".into())
            .await
            .unwrap(),
        ImportOutcome::Duplicate { stored_path: None }
    );
    assert_eq!(repository.albums().await.unwrap().len(), 1);
    assert!(
        artist::Entity::find()
            .all(&repository.db)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        album_artist::Entity::find()
            .all(&repository.db)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_concurrent_manual_add_only_receives_a_location() {
    let repository = repository().await;
    seed(&repository, "album", None).await;
    assert_eq!(
        repository
            .import(prepared("album"), "Artist/Title".into())
            .await
            .unwrap(),
        ImportOutcome::Location(LocationOutcome::Attached)
    );
    let stored = album::Entity::find_by_id("album")
        .one(&repository.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.title, "Original metadata");
    assert_eq!(stored.release_year, 1999);
    assert!(
        album_artist::Entity::find()
            .all(&repository.db)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repository
            .import(prepared("album"), "Artist/Title".into())
            .await
            .unwrap(),
        ImportOutcome::Location(LocationOutcome::Unchanged)
    );
    assert_eq!(
        repository
            .import(prepared("album"), "Artist/Other".into())
            .await
            .unwrap(),
        ImportOutcome::Duplicate {
            stored_path: Some("Artist/Title".into())
        }
    );
}

#[tokio::test]
async fn failed_credit_insertion_rolls_back_album_artists_and_location() {
    let repository = repository().await;
    repository.db.execute_unprepared("CREATE TRIGGER fail_credit BEFORE INSERT ON album_artist BEGIN SELECT RAISE(ABORT, 'test failure'); END").await.unwrap();
    assert!(
        repository
            .import(prepared("new"), "Artist/Title".into())
            .await
            .is_err()
    );
    assert!(repository.albums().await.unwrap().is_empty());
    assert!(
        artist::Entity::find()
            .all(&repository.db)
            .await
            .unwrap()
            .is_empty()
    );
}
