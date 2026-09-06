use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};

use super::*;

impl HasAlbumId for &str {
    fn album_id(&self) -> &str {
        self
    }
}

async fn test_database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    migration::Migrator::up(&db, None).await.unwrap();
    db
}

async fn insert_album(db: &DatabaseConnection, id: &str, relative_path: Option<&str>) {
    album::ActiveModel {
        id: Set(id.to_owned()),
        title: Set("Same title".to_owned()),
        release_year: Set(2026),
        relative_path: Set(relative_path.map(str::to_owned)),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn membership_uses_exact_ids_regardless_of_location_or_artist_credits() {
    let db = test_database().await;
    insert_album(&db, "undownloaded", None).await;
    insert_album(&db, "scanned", Some("Artist/Same title")).await;
    let albums = vec![
        "edition-2",
        "undownloaded",
        "scanned",
        "edition-1",
        "SCANNED",
    ];

    let eligible = exclude_catalog_albums(&db, albums).await.unwrap();

    assert_eq!(eligible, vec!["edition-2", "edition-1", "SCANNED"]);
}

#[tokio::test]
async fn empty_and_all_catalog_results_have_no_eligible_albums() {
    let db = test_database().await;
    insert_album(&db, "existing", None).await;

    let empty: Vec<&str> = exclude_catalog_albums(&db, Vec::<&str>::new())
        .await
        .unwrap();
    assert!(empty.is_empty());
    let all_catalog = exclude_catalog_albums(&db, vec!["existing"]).await.unwrap();
    assert!(all_catalog.is_empty());
}

#[tokio::test]
async fn later_load_checks_membership_again_without_changing_previous_results() {
    let db = test_database().await;
    let current_view = exclude_catalog_albums(&db, vec!["new"]).await.unwrap();
    insert_album(&db, "new", None).await;

    assert_eq!(current_view, vec!["new"]);
    let next_view = exclude_catalog_albums(&db, vec!["new"]).await.unwrap();
    assert!(next_view.is_empty());
}

#[tokio::test]
async fn membership_errors_fail_instead_of_showing_unchecked_results() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    assert_eq!(
        exclude_catalog_albums(&db, vec!["album"])
            .await
            .unwrap_err(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}
