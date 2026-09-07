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

fn discovery_album(
    album_id: &str,
    explicit: Option<bool>,
    media_tags: &[&str],
) -> services::tidal::TidalAlbum {
    services::tidal::TidalAlbum {
        id: album_id.to_owned(),
        title: "Same title".to_owned(),
        cover_url: None,
        release_date: Some("2026-01-01".to_owned()),
        popularity: 0.0,
        r#type: "ALBUM".to_owned(),
        explicit,
        media_tags: Some(media_tags.iter().map(|tag| (*tag).to_owned()).collect()),
    }
}

#[test]
fn discovery_ranking_orders_explicitness_then_quality_then_numeric_id() {
    let explicitness_by_preference = [Some(false), None, Some(true)];
    let media_tags_by_quality: &[&[&str]] =
        &[&[], &["DOLBY_ATMOS"], &["LOSSLESS"], &["HIRES_LOSSLESS"]];
    for (explicitness_rank, less_preferred_explicitness) in
        explicitness_by_preference.iter().enumerate()
    {
        for preferred_explicitness in &explicitness_by_preference[explicitness_rank + 1..] {
            for less_preferred_album_tags in media_tags_by_quality {
                for preferred_album_tags in media_tags_by_quality {
                    assert_winner(
                        discovery_album(
                            "100",
                            *less_preferred_explicitness,
                            less_preferred_album_tags,
                        ),
                        discovery_album("99", *preferred_explicitness, preferred_album_tags),
                        "99",
                    );
                }
            }
        }
    }
    for explicit in explicitness_by_preference {
        for (quality_rank, lower_quality_tags) in media_tags_by_quality.iter().enumerate() {
            for higher_quality_tags in &media_tags_by_quality[quality_rank + 1..] {
                assert_winner(
                    discovery_album("100", explicit, lower_quality_tags),
                    discovery_album("99", explicit, higher_quality_tags),
                    "99",
                );
            }
        }
        for matching_quality_tags in media_tags_by_quality {
            assert_winner(
                discovery_album("99", explicit, matching_quality_tags),
                discovery_album("100", explicit, matching_quality_tags),
                "100",
            );
        }
    }
    for lossless_tag in ["LOSSLESS", "HIRES_LOSSLESS"] {
        assert_winner(
            discovery_album("99", None, &[lossless_tag, "DOLBY_ATMOS"]),
            discovery_album("100", None, &[lossless_tag]),
            "100",
        );
    }
    let mut album_without_media_tags = discovery_album("100", None, &[]);
    album_without_media_tags.media_tags = None;
    assert_winner(
        album_without_media_tags,
        discovery_album("99", None, &["DOLBY_ATMOS"]),
        "99",
    );
}

fn assert_winner(
    lower_ranked_album: services::tidal::TidalAlbum,
    higher_ranked_album: services::tidal::TidalAlbum,
    expected_album_id: &str,
) {
    for candidate_order in [
        vec![lower_ranked_album.clone(), higher_ranked_album.clone()],
        vec![higher_ranked_album, lower_ranked_album],
    ] {
        let selected_albums = deduplicate_discovery_albums(candidate_order).unwrap();
        assert_eq!(selected_albums.len(), 1);
        assert_eq!(selected_albums[0].id, expected_album_id);
    }
}

#[test]
fn discovery_groups_by_normalized_title_and_release_type() {
    let lower_ranked_album = discovery_album("99", None, &[]);
    let mut higher_ranked_duplicate = discovery_album("100", None, &[]);
    higher_ranked_duplicate.title = " SAME title ".to_owned();
    let mut single = discovery_album("101", None, &[]);
    single.r#type = "SINGLE".to_owned();
    let mut deluxe = discovery_album("102", None, &[]);
    deluxe.title = "Same title (Deluxe)".to_owned();
    let mut album_with_distinct_punctuation = discovery_album("103", None, &[]);
    album_with_distinct_punctuation.title = "Same-title".to_owned();
    let selected_albums = deduplicate_discovery_albums(vec![
        lower_ranked_album,
        single,
        deluxe,
        album_with_distinct_punctuation,
        higher_ranked_duplicate,
    ])
    .unwrap();
    assert_eq!(
        selected_albums
            .iter()
            .map(|album| album.id.as_str())
            .collect::<Vec<_>>(),
        ["100", "101", "102", "103"]
    );
    assert!(deduplicate_discovery_albums(Vec::new()).unwrap().is_empty());
}

async fn credit_album(db: &DatabaseConnection, album_id: &str, artist_id: &str, position: i32) {
    crate::entity::artist::ActiveModel {
        id: Set(artist_id.to_owned()),
        name: Set(artist_id.to_owned()),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
    album_artist::ActiveModel {
        album_id: Set(album_id.to_owned()),
        artist_id: Set(artist_id.to_owned()),
        position: Set(position),
    }
    .insert(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn catalog_titles_suppress_all_types_for_any_credited_artist_until_deleted() {
    let db = test_database().await;
    insert_album(&db, "1", None).await;
    album::ActiveModel {
        id: Set("1".to_owned()),
        title: Set(" SAME title ".to_owned()),
        album_type: Set(Some("SINGLE".to_owned())),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    credit_album(&db, "1", "primary", 0).await;
    credit_album(&db, "1", "browsed", 1).await;
    insert_album(&db, "2", None).await;
    album_artist::ActiveModel {
        album_id: Set("2".to_owned()),
        artist_id: Set("browsed".to_owned()),
        position: Set(0),
    }
    .insert(&db)
    .await
    .unwrap();
    let mut single = discovery_album("101", Some(true), &["HIRES_LOSSLESS"]);
    single.r#type = "SINGLE".to_owned();
    let candidates = vec![
        discovery_album("100", Some(true), &["HIRES_LOSSLESS"]),
        single,
    ];
    assert!(
        select_discovery_albums(&db, "browsed", candidates.clone())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        select_discovery_albums(&db, "unrelated", candidates.clone())
            .await
            .unwrap()
            .len(),
        2
    );
    for deleted_album_id in ["1", "2"] {
        album_artist::Entity::delete_many()
            .filter(album_artist::Column::AlbumId.eq(deleted_album_id))
            .exec(&db)
            .await
            .unwrap();
        album::Entity::delete_by_id(deleted_album_id)
            .exec(&db)
            .await
            .unwrap();
        let discoverable_albums = select_discovery_albums(&db, "browsed", candidates.clone())
            .await
            .unwrap();
        assert_eq!(
            discoverable_albums.len(),
            if deleted_album_id == "1" { 0 } else { 2 }
        );
    }
}

#[tokio::test]
async fn discovery_catalog_errors_fail_instead_of_showing_unchecked_results() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    assert_eq!(
        select_discovery_albums(&db, "artist", vec![discovery_album("100", None, &[])])
            .await
            .unwrap_err(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
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
