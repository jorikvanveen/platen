use super::*;
use crate::entity::album;
use crate::{
    entity::{album_artist, artist},
    services::{
        catalog::{persist_album, prepare_album},
        tidal::{ResolvedTidalSearchedAlbum, TidalAlbum, TidalArtist, TidalError},
    },
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct Record {
    album: TidalAlbum,
    artists: Vec<TidalArtist>,
    search_title: Option<String>,
    search_date: Option<String>,
    search_artists: Option<Vec<TidalArtist>>,
    failure: Option<&'static str>,
}

fn record(id: &str, title: &str, date: &str) -> Record {
    Record {
        album: TidalAlbum {
            id: id.into(),
            title: title.into(),
            release_date: Some(date.into()),
            cover_url: None,
            popularity: 0.0,
            r#type: "ALBUM".into(),
            explicit: None,
            media_tags: None,
        },
        artists: vec![credit("primary", "AC/DC"), credit("guest", "Guest")],
        search_title: None,
        search_date: None,
        search_artists: None,
        failure: None,
    }
}

fn credit(id: &str, name: &str) -> TidalArtist {
    TidalArtist {
        id: id.into(),
        name: name.into(),
        profile_image_url: None,
    }
}

#[derive(Default)]
struct FakeCatalog {
    records: Vec<Record>,
    search_fails: bool,
    metadata_calls: AtomicUsize,
    artist_calls: AtomicUsize,
    cover_calls: AtomicUsize,
    gates: HashMap<String, Arc<Semaphore>>,
    active: AtomicUsize,
    peak: AtomicUsize,
}

impl FakeCatalog {
    fn assert_no_detail_calls(&self) {
        assert_eq!(self.metadata_calls.load(Ordering::SeqCst), 0, "get_album");
        assert_eq!(
            self.artist_calls.load(Ordering::SeqCst),
            0,
            "get_album_artists"
        );
        assert_eq!(
            self.cover_calls.load(Ordering::SeqCst),
            0,
            "get_album_cover"
        );
    }
}

#[async_trait::async_trait]
impl TidalCatalog for FakeCatalog {
    async fn find_album(&self, query: &str) -> Result<Vec<ResolvedTidalSearchedAlbum>, TidalError> {
        if self.search_fails {
            return Err(TidalError::UnexpectedResponse);
        }
        if let Some(gate) = self.gates.get(query) {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            gate.acquire().await.unwrap().forget();
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(self
            .records
            .iter()
            .map(|record| ResolvedTidalSearchedAlbum {
                id: record.album.id.clone(),
                title: record
                    .search_title
                    .clone()
                    .unwrap_or_else(|| record.album.title.clone()),
                cover_url: record.album.cover_url.clone(),
                release_date: record
                    .search_date
                    .clone()
                    .or_else(|| record.album.release_date.clone()),
                popularity: record.album.popularity,
                artists: record
                    .search_artists
                    .clone()
                    .unwrap_or_else(|| record.artists.clone()),
                r#type: record.album.r#type.clone(),
                explicit: record.album.explicit,
                media_tags: record.album.media_tags.clone(),
            })
            .collect())
    }

    async fn get_album(&self, id: &str) -> Result<TidalAlbum, TidalError> {
        self.metadata_calls.fetch_add(1, Ordering::SeqCst);
        let record = self
            .records
            .iter()
            .find(|record| record.album.id == id)
            .unwrap();
        if record.failure == Some("metadata") {
            return Err(TidalError::UnexpectedResponse);
        }
        Ok(record.album.clone())
    }

    async fn get_album_artists(&self, id: &str) -> Result<Vec<TidalArtist>, TidalError> {
        self.artist_calls.fetch_add(1, Ordering::SeqCst);
        let record = self
            .records
            .iter()
            .find(|record| record.album.id == id)
            .unwrap();
        if record.failure == Some("credits") {
            return Err(TidalError::UnexpectedResponse);
        }
        Ok(record.artists.clone())
    }

    async fn get_album_cover(&self, id: &str) -> Result<Option<String>, TidalError> {
        self.cover_calls.fetch_add(1, Ordering::SeqCst);
        let record = self
            .records
            .iter()
            .find(|record| record.album.id == id)
            .unwrap();
        if record.failure == Some("cover") {
            return Err(TidalError::UnexpectedResponse);
        }
        Ok(None)
    }
}

fn candidate(path: &str, artist: &str, title: &str, year: Option<i32>) -> AlbumCandidate {
    AlbumCandidate {
        primary_artist: artist.into(),
        title: title.into(),
        release_year: year,
        relative_path: path.into(),
    }
}

fn report(candidates: Vec<AlbumCandidate>) -> DiscoveryReport {
    DiscoveryReport {
        resolved_root: Some("/resolved/music".into()),
        candidates,
        ..Default::default()
    }
}

async fn database() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    <migration::Migrator as migration::MigratorTrait>::up(&db, None)
        .await
        .unwrap();
    db
}

async fn run(
    db: &DatabaseConnection,
    source: &dyn TidalCatalog,
    candidates: Vec<AlbumCandidate>,
) -> ScanSummary {
    let report = report(candidates);
    let repository = ImportRepository::new(db.clone());
    let mut progress = Progress::new(|_| {});
    progress.snapshot.summary = ScanSummary::from(&report);
    let plan = reconcile_catalog(&repository, &report, &mut progress)
        .await
        .unwrap();
    match_and_import(&repository, source, &report, plan, &mut progress).await;
    progress.snapshot.summary
}

#[tokio::test]
async fn conservative_identity_year_and_ordered_primary_artist() {
    for (artist, title, year, expected) in [
        (" ac - dc ", " a_b   (live) ", Some(2024), true),
        ("AC - DC", "A_B (Live)", None, true),
        ("AC/DC", "A:B (Live)", Some(2023), false),
        ("Guest", "A:B (Live)", None, false),
        ("ACDC", "A:B (Live)", None, false),
        ("AC/DC", "A:B", None, false),
        ("AC/DC", "A-B (Live)", None, false),
        ("AC/DC", "A:B (Deluxe)", None, false),
    ] {
        let source = FakeCatalog {
            records: vec![record("one", "A:B (Live)", "2024-02-29")],
            ..Default::default()
        };
        let result =
            match_candidate(&source, &candidate("Artist/Album", artist, title, year)).await;
        assert_eq!(
            matches!(result, MatchOutcome::Unique(_)),
            expected,
            "{artist} {title} {year:?}: {result:?}"
        );
    }
}

#[tokio::test]
async fn dates_must_parse_and_match_in_search_metadata_only() {
    for (search_date, metadata_date, year, expected) in [
        ("2024", "2024", Some(2024), "unique"),
        ("2024-02", "2024-02-29", Some(2024), "unique"),
        ("2023", "2024", Some(2024), "unmatched"),
        ("2024", "2023", Some(2024), "unique"),
        ("2024-02-30", "2024", None, "failed"),
        ("2024", "2024-02-30", None, "unique"),
        ("", "2024", None, "failed"),
    ] {
        let mut album = record("one", "Title", metadata_date);
        album.search_date = Some(search_date.into());
        let source = FakeCatalog {
            records: vec![album],
            ..Default::default()
        };
        let outcome =
            match_candidate(&source, &candidate("Artist/Title", "AC/DC", "Title", year)).await;
        assert_eq!(
            outcome_name(&outcome),
            expected,
            "{search_date} {metadata_date} {year:?}"
        );
        source.assert_no_detail_calls();
    }
}

fn outcome_name(outcome: &MatchOutcome) -> &'static str {
    match outcome {
        MatchOutcome::Unique(_) => "unique",
        MatchOutcome::Unmatched => "unmatched",
        MatchOutcome::Ambiguous(_) => "ambiguous",
        MatchOutcome::Failed => "failed",
    }
}

#[tokio::test]
async fn uniqueness_requires_valid_search_metadata_and_distinct_ids() {
    let first = record("one", "Title", "2024");
    let later = record("two", "Title", "2025");
    let mut wrong_title = record("two", "Title (Deluxe)", "2024");
    wrong_title.search_title = Some("Title".into());
    let mut wrong_primary = record("two", "Title", "2024");
    wrong_primary.search_artists = Some(wrong_primary.artists.clone());
    wrong_primary.artists.reverse();
    let mut broken = record("two", "Title", "2024");
    broken.search_artists = Some(vec![credit("primary", "AC/DC"), credit("", "Guest")]);
    for (records, year, expected) in [
        (vec![], None, "unmatched"),
        (vec![first.clone(), first.clone()], None, "unique"),
        (vec![first.clone(), later.clone()], None, "ambiguous"),
        (vec![first.clone(), later], Some(2024), "unique"),
        (
            vec![first.clone(), record("two", "Title", "2024")],
            Some(2024),
            "ambiguous",
        ),
        (vec![first.clone(), wrong_title], None, "ambiguous"),
        (vec![first.clone(), wrong_primary], None, "ambiguous"),
        (vec![first, broken], None, "failed"),
    ] {
        let source = FakeCatalog {
            records,
            ..Default::default()
        };
        let outcome =
            match_candidate(&source, &candidate("Artist/Title", "AC/DC", "Title", year)).await;
        assert_eq!(outcome_name(&outcome), expected);
        if let MatchOutcome::Ambiguous(ids) = outcome {
            assert_eq!(ids, vec!["one", "two"]);
        }
        source.assert_no_detail_calls();
    }
}

#[tokio::test]
async fn invalid_search_metadata_and_candidate_failures_do_not_import() {
    for failure in [
        "search",
        "empty_credits",
        "blank_id",
        "blank_title",
        "missing_date",
        "invalid_date",
        "blank_primary_name",
        "blank_primary_id",
        "blank_guest_name",
        "blank_guest_id",
        "duplicate_artist_ids",
    ] {
        let mut album = record("one", "Title", "2024");
        let mut artists = album.artists.clone();
        match failure {
            "empty_credits" => artists.clear(),
            "blank_id" => album.album.id = " ".into(),
            "blank_title" => album.search_title = Some(" ".into()),
            "missing_date" => album.album.release_date = None,
            "invalid_date" => album.search_date = Some("2024-02-30".into()),
            "blank_primary_name" => artists[0].name = " ".into(),
            "blank_primary_id" => artists[0].id = " ".into(),
            "blank_guest_name" => artists[1].name = " ".into(),
            "blank_guest_id" => artists[1].id = " ".into(),
            "duplicate_artist_ids" => artists[1].id = artists[0].id.clone(),
            _ => {}
        }
        album.search_artists = Some(artists);
        let source = FakeCatalog {
            records: vec![album],
            search_fails: failure == "search",
            ..Default::default()
        };
        let db = database().await;
        let summary = run(
            &db,
            &source,
            vec![candidate("Artist/Title", "AC/DC", "Title", None)],
        )
        .await;
        assert_eq!(summary.albums_imported, 0, "{failure}");
        assert_eq!(summary.failures, 1, "{failure}");
        assert!(
            album::Entity::find().all(&db).await.unwrap().is_empty(),
            "{failure}"
        );
        assert_eq!(summary.skipped_directories, summary.failures);
        assert_eq!(summary.candidates_processed, 1);
        source.assert_no_detail_calls();
    }
}

#[tokio::test]
async fn imports_preserve_search_data_without_any_detail_requests() {
    for failure in [None, Some("metadata"), Some("credits"), Some("cover")] {
        let db = database().await;
        let mut album = record("one", "Detail title", "1999");
        album.search_title = Some("Search title".into());
        album.search_date = Some("2024-02-29".into());
        album.album.cover_url = Some("https://example.com/search-cover.jpg".into());
        album.album.popularity = 0.75;
        album.album.r#type = "EP".into();
        let mut primary = credit("search-primary", "AC/DC");
        primary.profile_image_url = Some("https://example.com/primary.jpg".into());
        let mut guest = credit("search-guest", "Search guest");
        guest.profile_image_url = Some("https://example.com/guest.jpg".into());
        let search_artists = vec![primary, guest];
        album.search_artists = Some(search_artists.clone());
        album.artists = vec![credit("detail-primary", "Different artist")];
        album.failure = failure;
        let source = FakeCatalog {
            records: vec![album],
            ..Default::default()
        };
        let candidate = candidate("Artist/Search title", "AC/DC", "Search title", Some(2024));
        let outcome = match_candidate(&source, &candidate).await;
        source.assert_no_detail_calls();
        let MatchOutcome::Unique(prepared) = outcome else {
            panic!("Expected unique search match with {failure:?}, got {outcome:?}");
        };
        assert_eq!(prepared.album().popularity, 0.75);
        assert_eq!(prepared.album().release_date.as_deref(), Some("2024-02-29"));

        let summary = run(&db, &source, vec![candidate]).await;
        source.assert_no_detail_calls();
        assert_eq!(summary.albums_imported, 1, "{failure:?}");
        assert_eq!(summary.failures, 0);
        assert_eq!(summary.skipped_directories, 0);
        let stored = album::Entity::find_by_id("one")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.title, "Search title");
        assert_eq!(stored.album_type.as_deref(), Some("EP"));
        assert_eq!(stored.release_year, 2024);
        assert_eq!(stored.release_month, Some(2));
        assert_eq!(stored.release_day, Some(29));
        assert_eq!(
            stored.cover_url.as_deref(),
            Some("https://example.com/search-cover.jpg")
        );
        assert_eq!(stored.relative_path.as_deref(), Some("Artist/Search title"));
        let credits = album_artist::Entity::find()
            .filter(album_artist::Column::AlbumId.eq("one"))
            .order_by_asc(album_artist::Column::Position)
            .all(&db)
            .await
            .unwrap();
        assert_eq!(credits.len(), search_artists.len());
        for (position, (credit, expected)) in credits.iter().zip(&search_artists).enumerate() {
            assert_eq!(credit.artist_id, expected.id);
            assert_eq!(credit.position, position as i32);
            let stored_artist = artist::Entity::find_by_id(&credit.artist_id)
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored_artist.name, expected.name);
            assert_eq!(stored_artist.profile_image_url, expected.profile_image_url);
        }
        assert_eq!(artist::Entity::find().all(&db).await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn scan_preserves_explicitness_and_raw_tags_without_refresh_or_detail_requests() {
    for explicit in [Some(true), Some(false), None] {
        for tags in [
            None,
            Some(vec![]),
            Some(vec!["FUTURE"]),
            Some(vec!["DOLBY_ATMOS"]),
            Some(vec!["LOSSLESS", "HIRES_LOSSLESS", "DOLBY_ATMOS", "FUTURE"]),
        ] {
            let db = database().await;
            let mut record = record("one", "Title", "2024");
            record.album.explicit = explicit;
            record.album.media_tags = tags
                .as_ref()
                .map(|tags| tags.iter().map(|tag| (*tag).to_owned()).collect());
            let mut source = FakeCatalog {
                records: vec![record],
                ..Default::default()
            };
            let candidates = vec![candidate("Artist/Title", "AC/DC", "Title", Some(2024))];
            let summary = run(&db, &source, candidates.clone()).await;
            assert_eq!(summary.albums_imported, 1);
            let stored = album::Entity::find_by_id("one")
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.explicit, explicit);
            assert_eq!(stored.media_tags, tags.map(serde_json::Value::from));
            source.records[0].album.explicit = Some(!explicit.unwrap_or(false));
            source.records[0].album.media_tags = Some(vec!["REPLACEMENT".into()]);
            assert_eq!(run(&db, &source, candidates).await.albums_imported, 0);
            assert_eq!(
                album::Entity::find_by_id("one")
                    .one(&db)
                    .await
                    .unwrap()
                    .unwrap(),
                stored
            );
            source.assert_no_detail_calls();
        }
    }
}

#[tokio::test]
async fn duplicate_locations_skip_all_copies_in_any_order() {
    for stored_path in [None, Some("Old/Title")] {
        for reverse in [false, true] {
            let db = database().await;
            if let Some(path) = stored_path {
                album::ActiveModel {
                    id: Set("one".into()),
                    title: Set("Old title".into()),
                    release_year: Set(1999),
                    relative_path: Set(Some(path.into())),
                    ..Default::default()
                }
                .insert(&db)
                .await
                .unwrap();
            }
            let source = FakeCatalog {
                records: vec![record("one", "Title", "2024")],
                ..Default::default()
            };
            let mut candidates = vec![
                candidate("Artist/Title", "AC/DC", "Title", None),
                candidate("Artist/Title (2024)", "AC/DC", "Title", Some(2024)),
            ];
            if let Some(path) = stored_path {
                candidates.push(candidate(path, "Old", "Title", None));
            }
            if reverse {
                candidates.reverse();
            }
            let summary = run(&db, &source, candidates).await;
            assert_eq!(
                summary.duplicate_locations,
                2 + usize::from(stored_path.is_some())
            );
            assert_eq!(summary.skipped_directories, summary.duplicate_locations);
            assert_eq!(summary.albums_imported + summary.locations_attached, 0);
            let stored = album::Entity::find_by_id("one").one(&db).await.unwrap();
            assert_eq!(
                stored
                    .as_ref()
                    .and_then(|album| album.relative_path.as_deref()),
                stored_path
            );
            assert_eq!(stored.is_some(), stored_path.is_some());
        }
    }
}

#[tokio::test]
async fn a_tidal_alias_does_not_double_count_known_duplicate_paths() {
    let db = database().await;
    let original = FakeCatalog {
        records: vec![record("one", "Old", "2024")],
        ..Default::default()
    };
    persist_album(
        &db,
        prepare_album(&original, "one").await.unwrap(),
        Some("Artist/Old".into()),
    )
    .await
    .unwrap();
    let source = FakeCatalog {
        records: vec![record("one", "New", "2024")],
        ..Default::default()
    };
    let summary = run(
        &db,
        &source,
        vec![
            candidate("Artist/Old", "AC/DC", "Old", None),
            candidate("Artist/Old (2024)", "AC/DC", "Old", Some(2024)),
            candidate("Artist/New", "AC/DC", "New", None),
        ],
    )
    .await;
    assert_eq!(summary.candidates_processed, 3);
    assert_eq!(summary.duplicate_locations, 3);
    assert_eq!(summary.skipped_directories, 3);
    assert_eq!(summary.unchanged_locations, 1);
    assert_eq!(summary.albums_imported + summary.locations_attached, 0);
    let stored = album::Entity::find_by_id("one")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.title, "Old");
    assert_eq!(stored.relative_path.as_deref(), Some("Artist/Old"));
}

#[tokio::test]
async fn known_duplicates_without_a_stored_location_block_every_tidal_alias() {
    for stored_path in [None, Some("Missing/Old")] {
        for reverse in [false, true] {
            for alias_count in [1, 2] {
                let db = database().await;
                let original = FakeCatalog {
                    records: vec![record("one", "Old", "2024")],
                    ..Default::default()
                };
                persist_album(
                    &db,
                    prepare_album(&original, "one").await.unwrap(),
                    stored_path.map(str::to_owned),
                )
                .await
                .unwrap();
                let source = FakeCatalog {
                    records: vec![record("one", "New", "2024")],
                    ..Default::default()
                };
                let mut candidates = vec![
                    candidate("Artist/Old", "AC/DC", "Old", None),
                    candidate("Artist/Old (2024)", "AC/DC", "Old", Some(2024)),
                    candidate("Artist/New", "AC/DC", "New", None),
                ];
                if alias_count == 2 {
                    candidates.push(candidate("Artist/New (2024)", "AC/DC", "New", Some(2024)));
                }
                if reverse {
                    candidates.reverse();
                }
                let summary = run(&db, &source, candidates).await;
                assert_eq!(summary.candidates_processed, 2 + alias_count);
                assert_eq!(summary.duplicate_locations, 2 + alias_count);
                assert_eq!(summary.skipped_directories, 2 + alias_count);
                assert_eq!(
                    summary.albums_imported
                        + summary.locations_attached
                        + summary.locations_changed,
                    0
                );
                assert_eq!(
                    summary.locations_cleared,
                    usize::from(stored_path.is_some())
                );
                assert_eq!(
                    summary.failures + summary.unmatched_candidates + summary.ambiguous_matches,
                    0
                );
                let stored = album::Entity::find_by_id("one")
                    .one(&db)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(stored.title, "Old");
                assert_eq!(stored.relative_path, None);
            }
        }
    }
}

#[tokio::test]
async fn existing_ids_attach_without_refresh_and_imports_keep_credit_order() {
    let db = database().await;
    album::ActiveModel {
        id: Set("existing".into()),
        title: Set("Keep metadata".into()),
        release_year: Set(1999),
        cover_url: Set(Some("old-cover".into())),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let before = album::Entity::find_by_id("existing")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let source = FakeCatalog {
        records: vec![
            record("existing", "Title", "2024"),
            record("new", "New", "2024-02-29"),
        ],
        ..Default::default()
    };
    let candidates = vec![
        candidate("Artist/Title", "AC/DC", "Title", None),
        candidate("Artist/New", "AC/DC", "New", None),
    ];
    let summary = run(&db, &source, candidates.clone()).await;
    assert_eq!(summary.locations_attached, 1);
    assert_eq!(summary.albums_imported, 1);
    let mut after = album::Entity::find_by_id("existing")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.relative_path.as_deref(), Some("Artist/Title"));
    after.relative_path = None;
    assert_eq!(before, after);
    let credits = album_artist::Entity::find()
        .order_by_asc(album_artist::Column::Position)
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        credits
            .iter()
            .map(|credit| (
                credit.album_id.as_str(),
                credit.artist_id.as_str(),
                credit.position
            ))
            .collect::<Vec<_>>(),
        vec![("new", "primary", 0), ("new", "guest", 1)]
    );
    source.assert_no_detail_calls();
    let second = run(&db, &source, candidates).await;
    assert_eq!(second.unchanged_locations, 2);
    assert_eq!(second.albums_imported + second.locations_attached, 0);
    source.assert_no_detail_calls();
}

#[tokio::test]
async fn failed_import_rolls_back_independently_and_occupied_paths_are_skipped() {
    let db = database().await;
    db.execute_unprepared("CREATE TRIGGER fail_credit BEFORE INSERT ON album_artist WHEN NEW.album_id = 'bad' BEGIN SELECT RAISE(ABORT, 'test credit failure'); END").await.unwrap();
    let source = FakeCatalog {
        records: vec![record("bad", "Bad", "2024"), record("good", "Good", "2024")],
        ..Default::default()
    };
    let summary = run(
        &db,
        &source,
        vec![
            candidate("Artist/Bad", "AC/DC", "Bad", None),
            candidate("Artist/Good", "AC/DC", "Good", None),
        ],
    )
    .await;
    assert_eq!(summary.failures, 1);
    assert_eq!(summary.albums_imported, 1);
    assert!(
        album::Entity::find_by_id("bad")
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(artist::Entity::find().all(&db).await.unwrap().len(), 2);
    let prepared = prepare_album(&source, "bad").await.unwrap();
    let outcome = ImportRepository::new(db.clone())
        .import(prepared, "Artist/Good".into())
        .await
        .unwrap();
    assert_eq!(outcome, ImportOutcome::Duplicate { stored_path: None });
    assert!(
        album::Entity::find_by_id("bad")
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn matching_is_bounded_publishes_each_completion_and_defers_all_imports() {
    let db = database().await;
    let first = Arc::new(Semaphore::new(0));
    let second = Arc::new(Semaphore::new(0));
    let third = Arc::new(Semaphore::new(0));
    let source = Arc::new(FakeCatalog {
        records: vec![
            record("one", "One", "2024"),
            record("two", "Two", "2024"),
            record("three", "Three", "2024"),
        ],
        gates: HashMap::from([
            ("AC/DC One".into(), first.clone()),
            ("AC/DC Two".into(), second.clone()),
            ("AC/DC Three".into(), third.clone()),
        ]),
        ..Default::default()
    });
    let report = report(vec![
        candidate("Artist/One", "AC/DC", "One", None),
        candidate("Artist/Two", "AC/DC", "Two", None),
        candidate("Artist/Three", "AC/DC", "Three", None),
    ]);
    let (publisher, handle) = tokio::sync::watch::channel(ScanSnapshot::default());
    let task = tokio::spawn({
        let db = db.clone();
        let source = source.clone();
        async move {
            let repository = ImportRepository::new(db);
            let mut progress = Progress::new(move |snapshot| {
                publisher.send_replace(snapshot);
            });
            progress.snapshot.summary = ScanSummary::from(&report);
            let plan = reconcile_catalog(&repository, &report, &mut progress)
                .await
                .unwrap();
            match_and_import(&repository, source.as_ref(), &report, plan, &mut progress).await;
            progress.snapshot.summary
        }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while source.active.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
        assert_eq!(handle.borrow().clone().summary.candidates_processed, 0);
        first.add_permits(1);
        while handle.borrow().clone().summary.candidates_processed != 1 {
            tokio::task::yield_now().await;
        }
        assert!(album::Entity::find().all(&db).await.unwrap().is_empty());
        third.add_permits(1);
        while handle.borrow().clone().summary.candidates_processed != 2 {
            tokio::task::yield_now().await;
        }
        assert!(album::Entity::find().all(&db).await.unwrap().is_empty());
        second.add_permits(1);
    })
    .await
    .unwrap();
    let summary = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.candidates_processed, 3);
    assert_eq!(summary.albums_imported, 3);
    assert_eq!(source.peak.load(Ordering::SeqCst), 2);
    assert_eq!(handle.borrow().clone().summary, summary);
}
