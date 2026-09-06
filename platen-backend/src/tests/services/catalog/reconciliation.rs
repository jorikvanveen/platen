use super::super::scan::FilesystemDiagnostic;
use super::*;
use migration::MigratorTrait;
use sea_orm::{ColumnTrait, Database, QueryFilter};

#[test]
fn location_decisions_require_a_unique_candidate_unless_the_old_path_is_observed() {
    let first = candidate("Artist/Title", "Artist", "Title", None);
    let second = candidate("Artist/Title (2024)", "Artist", "Title", Some(2024));
    let candidates = [&first, &second];
    for (old_path, observed, expected) in [
        (
            None,
            false,
            [
                LocationDecision::NoLocation,
                LocationDecision::Attach("Artist/Title"),
                LocationDecision::NoLocation,
            ],
        ),
        (
            Some("Old/Title"),
            false,
            [
                LocationDecision::Clear,
                LocationDecision::Change("Artist/Title"),
                LocationDecision::Clear,
            ],
        ),
        (
            Some("Old/Title"),
            true,
            [
                LocationDecision::Keep("Old/Title"),
                LocationDecision::Keep("Old/Title"),
                LocationDecision::Keep("Old/Title"),
            ],
        ),
    ] {
        for (candidate_count, expected) in expected.into_iter().enumerate() {
            assert_eq!(
                decide_location(old_path, observed, &candidates[..candidate_count]),
                expected,
                "old_path={old_path:?}, observed={observed}, candidates={candidate_count}"
            );
        }
    }
}

async fn database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    migration::Migrator::up(&db, None).await.unwrap();
    db
}

async fn seed(
    db: &DatabaseConnection,
    id: &str,
    title: &str,
    year: i32,
    path: Option<&str>,
    artists: &[&str],
) {
    album::ActiveModel {
        id: Set(id.into()),
        title: Set(title.into()),
        release_year: Set(year),
        relative_path: Set(path.map(str::to_owned)),
        album_type: Set(Some("ALBUM".into())),
        release_month: Set(Some(6)),
        release_day: Set(Some(12)),
        cover_url: Set(Some("https://example.test/cover".into())),
    }
    .insert(db)
    .await
    .unwrap();
    for (position, name) in artists.iter().enumerate().rev() {
        let artist_id = format!("{id}-{position}");
        artist::ActiveModel {
            id: Set(artist_id.clone()),
            name: Set((*name).into()),
            profile_image_url: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        album_artist::ActiveModel {
            album_id: Set(id.into()),
            artist_id: Set(artist_id),
            position: Set(position as i32),
        }
        .insert(db)
        .await
        .unwrap();
    }
}

fn candidate(path: &str, artist: &str, title: &str, year: Option<i32>) -> AlbumCandidate {
    AlbumCandidate {
        relative_path: path.into(),
        primary_artist: artist.into(),
        title: title.into(),
        release_year: year,
    }
}

async fn stored(db: &DatabaseConnection, id: &str) -> album::Model {
    album::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

async fn run(db: &DatabaseConnection, candidates: Vec<AlbumCandidate>) -> ScanSummary {
    let mut report = DiscoveryReport {
        candidates,
        ..Default::default()
    };
    let mut summary = ScanSummary::from(&report);
    reconcile(db, &mut report, &mut summary).await.unwrap();
    summary
}

#[tokio::test]
async fn unknown_candidates_are_returned_without_counting_them_as_processed_or_skipped() {
    let db = database().await;
    seed(&db, "known", "Known", 2024, None, &["Artist"]).await;
    let unknown = candidate("Artist/Unknown", "Artist", "Unknown", None);
    let mut report = DiscoveryReport {
        candidates: vec![
            candidate("Artist/Known", "Artist", "Known", None),
            unknown.clone(),
        ],
        ..Default::default()
    };
    let mut summary = ScanSummary::from(&report);
    assert_eq!(
        reconcile(&db, &mut report, &mut summary).await.unwrap(),
        vec![unknown]
    );
    assert_eq!(summary.candidates_total, 2);
    assert_eq!(summary.candidates_processed, 1);
    assert_eq!(summary.locations_attached, 1);
    assert_eq!(summary.unmatched_candidates, 0);
    assert_eq!(summary.skipped_directories, 0);
}

#[tokio::test]
async fn attaches_moves_keeps_and_clears_without_refreshing_metadata_or_credits() {
    let db = database().await;
    seed(&db, "attach", "Attach", 2024, None, &["Artist", "Guest"]).await;
    seed(&db, "move", "Move", 2024, Some("Old/Move"), &["Artist"]).await;
    seed(
        &db,
        "keep",
        "Renamed metadata",
        2024,
        Some("Artist/Keep"),
        &["Other artist"],
    )
    .await;
    seed(
        &db,
        "clear",
        "Clear",
        2024,
        Some("Artist/Clear"),
        &["Artist"],
    )
    .await;
    seed(
        &db,
        "no-credit",
        "Missing credits",
        2024,
        Some("Artist/Missing credits"),
        &[],
    )
    .await;
    let before = album::Entity::find().all(&db).await.unwrap();
    let credits = album_artist::Entity::find().all(&db).await.unwrap();
    let artists = artist::Entity::find().all(&db).await.unwrap();
    let candidates = vec![
        candidate("Artist/Attach (2024)", "Artist", "Attach", Some(2024)),
        candidate("Artist/Move", "Artist", "Move", None),
        candidate("Artist/Keep", "Artist", "Keep", None),
    ];
    let summary = run(&db, candidates.clone()).await;
    assert_eq!(summary.locations_attached, 1);
    assert_eq!(summary.locations_changed, 1);
    assert_eq!(summary.unchanged_locations, 1);
    assert_eq!(summary.locations_cleared, 2);
    assert_eq!(summary.failures, 0);
    assert_eq!(
        stored(&db, "attach").await.relative_path.as_deref(),
        Some("Artist/Attach (2024)")
    );
    assert_eq!(
        stored(&db, "move").await.relative_path.as_deref(),
        Some("Artist/Move")
    );
    for original in before {
        let mut after = stored(&db, &original.id).await;
        after.relative_path = original.relative_path.clone();
        assert_eq!(after, original);
    }
    assert_eq!(
        album_artist::Entity::find().all(&db).await.unwrap(),
        credits
    );
    assert_eq!(artist::Entity::find().all(&db).await.unwrap(), artists);
    let second = run(&db, candidates).await;
    assert_eq!(second.unchanged_locations, 3);
    assert_eq!(
        second.locations_attached + second.locations_changed + second.locations_cleared,
        0
    );
}

#[tokio::test]
async fn supplied_permission_diagnostic_does_not_preserve_any_absent_path() {
    let db = database().await;
    seed(
        &db,
        "hidden",
        "Hidden",
        2024,
        Some("Artist/Hidden"),
        &["Artist"],
    )
    .await;
    seed(&db, "gone", "Gone", 2024, Some("Other/Gone"), &["Other"]).await;
    seed(
        &db,
        "visible",
        "Visible",
        2024,
        Some("Artist/Visible"),
        &["Artist"],
    )
    .await;
    let mut report = DiscoveryReport {
        candidates: vec![candidate("Artist/Visible", "Artist", "Visible", None)],
        diagnostics: vec![FilesystemDiagnostic {
            reason: "read_directory",
            path: "/resolved/music/Artist/Hidden".into(),
            os_error: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        }],
        root_failed: false,
        ..Default::default()
    };
    let mut summary = ScanSummary::from(&report);
    reconcile(&db, &mut report, &mut summary).await.unwrap();
    assert_eq!(summary.filesystem_errors, 1);
    assert_eq!(summary.locations_cleared, 2);
    assert_eq!(summary.unchanged_locations, 1);
    assert_eq!(stored(&db, "hidden").await.relative_path, None);
    assert_eq!(stored(&db, "gone").await.relative_path, None);
    assert_eq!(
        stored(&db, "visible").await.relative_path.as_deref(),
        Some("Artist/Visible")
    );
}

#[tokio::test]
async fn duplicates_skip_every_copy_and_only_retain_an_observed_old_location() {
    for old_path in [None, Some("Artist/Title"), Some("Missing/Title")] {
        let db = database().await;
        seed(&db, "album", "Title", 2024, old_path, &["Artist"]).await;
        let copies = vec![
            candidate("Artist/Title", "Artist", "Title", None),
            candidate("Artist/Title (2024)", "Artist", "Title", Some(2024)),
        ];
        for candidates in [copies.clone(), copies.into_iter().rev().collect()] {
            let summary = run(&db, candidates).await;
            assert_eq!(summary.duplicate_locations, 2);
            assert_eq!(summary.skipped_directories, 2);
            assert_eq!(summary.locations_attached + summary.locations_changed, 0);
            let expected = old_path.filter(|path| *path == "Artist/Title");
            assert_eq!(
                stored(&db, "album").await.relative_path.as_deref(),
                expected
            );
            assert_eq!(summary.unchanged_locations, usize::from(expected.is_some()));
        }
    }
}

#[tokio::test]
async fn observed_old_name_and_new_match_are_both_duplicates() {
    let db = database().await;
    seed(
        &db,
        "album",
        "New title",
        2024,
        Some("Old artist/Old title"),
        &["Artist"],
    )
    .await;
    let summary = run(
        &db,
        vec![
            candidate("Old artist/Old title", "Old artist", "Old title", None),
            candidate("Artist/New title", "Artist", "New title", None),
        ],
    )
    .await;
    assert_eq!(summary.duplicate_locations, 2);
    assert_eq!(summary.unchanged_locations, 1);
    assert_eq!(
        stored(&db, "album").await.relative_path.as_deref(),
        Some("Old artist/Old title")
    );
}

#[tokio::test]
async fn yearless_and_same_year_ambiguity_never_choose_an_album() {
    let db = database().await;
    seed(&db, "early", "Title", 2023, Some("Gone/Title"), &["Artist"]).await;
    seed(&db, "later", "Title", 2024, None, &["Artist"]).await;
    let summary = run(
        &db,
        vec![candidate("Artist/Title", "Artist", "Title", None)],
    )
    .await;
    assert_eq!(summary.ambiguous_matches, 1);
    assert_eq!(summary.locations_cleared, 1);
    let summary = run(
        &db,
        vec![candidate(
            "Artist/Title (2024)",
            "Artist",
            "Title",
            Some(2024),
        )],
    )
    .await;
    assert_eq!(summary.locations_attached, 1);
    assert_eq!(
        stored(&db, "later").await.relative_path.as_deref(),
        Some("Artist/Title (2024)")
    );
    seed(&db, "same-year", "Title", 2024, None, &["Artist"]).await;
    let summary = run(
        &db,
        vec![candidate(
            "ARTIST/Title (2024)",
            "Artist",
            "Title",
            Some(2024),
        )],
    )
    .await;
    assert_eq!(summary.ambiguous_matches, 1);
    assert_eq!(summary.locations_cleared, 1);
    assert!(
        album::Entity::find()
            .filter(album::Column::RelativePath.is_not_null())
            .all(&db)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn matching_uses_shared_conversion_and_only_conservative_normalization() {
    for (artist, title, year, matches) in [
        (" ac - dc ", " a_b   (live) ", Some(2024), true),
        ("AC - DC", "A_B (Live)", None, true),
        ("AC - DC", "A_B (Live)", Some(2023), false),
        ("Guest", "A_B (Live)", Some(2024), false),
        ("ACDC", "A_B (Live)", Some(2024), false),
        ("AC - DC", "A_B", Some(2024), false),
        ("AC - DC", "A-B (Live)", Some(2024), false),
    ] {
        let db = database().await;
        seed(&db, "album", "A:B (Live)", 2024, None, &["AC/DC", "Guest"]).await;
        let summary = run(&db, vec![candidate("Candidate/Album", artist, title, year)]).await;
        assert_eq!(
            summary.locations_attached,
            usize::from(matches),
            "{artist} / {title} / {year:?}"
        );
        assert_eq!(summary.unmatched_candidates, 0);
        assert_eq!(summary.candidates_processed, usize::from(matches));
        assert_eq!(summary.skipped_directories, 0);
        assert_eq!(stored(&db, "album").await.relative_path.is_some(), matches);
    }
}
