use super::*;
use reqwest::StatusCode;
use serde_json::{Value, json};

struct Fixture {
    state: AppState,
    worker: tokio::task::JoinHandle<()>,
}

impl Fixture {
    fn new(db: DatabaseConnection) -> Self {
        let (queue, worker) = DownloadQueue::start(
            db.clone(),
            MusicDirectory::new(temp_music_dir()),
            GateDownloader::new(),
        );
        Self {
            state: app_state(db, queue),
            worker,
        }
    }

    fn app(&self, albums: Vec<ScanAlbum>) -> Router {
        let mut state = self.state.clone();
        state.tidal = Arc::new(FakeTidalCatalog {
            albums,
            ..Default::default()
        });
        router(state)
    }

    async fn discover(&self, albums: Vec<ScanAlbum>, artist_id: &str) -> Value {
        let (status, body) = request(
            self.app(albums),
            "GET",
            &format!("/tidal/artists/{artist_id}"),
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        body
    }

    async fn add(&self, record: ScanAlbum) {
        let album_id = record.album.id.clone();
        let (status, body) = request(
            self.app(vec![record]),
            "POST",
            &format!("/albums/{album_id}"),
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["id"], album_id);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

async fn request(app: Router, method: &str, path: &str, body: &str) -> (StatusCode, Value) {
    let response = send_request(app, method, path, body).await;
    let status = response.status();
    if status.is_success() {
        assert_eq!(response.headers()["content-type"], "application/json");
    }
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body = if status.is_success() {
        serde_json::from_slice(&bytes).unwrap()
    } else {
        Value::String(String::from_utf8(bytes.to_vec()).unwrap())
    };
    (status, body)
}

fn candidate(id: &str, title: &str, release_type: &str) -> ScanAlbum {
    let mut record = ScanAlbum::new(id, title, "2026-01-01");
    record.album.r#type = release_type.to_owned();
    record
}

fn dated_candidate(id: &str, title: &str, release_type: &str, release_date: &str) -> ScanAlbum {
    let mut record = candidate(id, title, release_type);
    record.album.release_date = Some(release_date.to_owned());
    record
}

fn album_ids(body: &Value) -> Vec<&str> {
    body["albums"]
        .as_array()
        .unwrap()
        .iter()
        .map(|album| album["id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn discovery_normalizes_titles_without_changing_types_metadata_or_group_order() {
    let fixture = Fixture::new(test_database().await);
    let mut winner = candidate("100", "\t SAME  \n title\u{00a0}", "ALBUM");
    winner.album.cover_url = Some("https://example.test/cover.jpg".to_owned());
    // A shared release date keeps the newest-first sort out of the group order
    // this test asserts.
    winner.album.popularity = 0.5;
    winner.album.explicit = Some(true);
    winner.album.media_tags = Some(vec!["LOSSLESS".to_owned()]);
    let result = fixture
        .discover(
            vec![
                candidate("99", "same title", "ALBUM"),
                candidate("101", "same title", "SINGLE"),
                candidate("102", "same title", "EP"),
                candidate("103", "same title (Deluxe)", "ALBUM"),
                candidate("104", "same-title", "ALBUM"),
                winner,
            ],
            "a-guest",
        )
        .await;

    assert_eq!(result["artist"]["id"], "a-guest");
    assert_eq!(result["returned_count"], 6);
    assert_eq!(album_ids(&result), ["100", "101", "102", "103", "104"]);
    assert_eq!(
        result["albums"][0],
        json!({
            "id": "100",
            "title": "\t SAME  \n title\u{00a0}",
            "cover_url": "https://example.test/cover.jpg",
            "album_type": "ALBUM",
            "release_date": "2026-01-01",
            "popularity": 0.5,
            "explicit": true,
            "media_tags": ["LOSSLESS"],
            "available_quality": "LOSSLESS"
        })
    );
    assert_eq!(result["albums"][1]["album_type"], "SINGLE");
    assert_eq!(result["albums"][2]["album_type"], "EP");
}

#[tokio::test]
async fn discovery_orders_albums_newest_first_instead_of_by_tidal_order() {
    let fixture = Fixture::new(test_database().await);
    // The larger id sits earlier in Tidal's order, so a stable undated tail is
    // distinguishable from sorting the tail by id.
    let mut undated_early_position = candidate("90", "Undated sessions", "ALBUM");
    undated_early_position.album.release_date = None;
    let mut undated_late_position = candidate("89", "Lost tapes", "ALBUM");
    undated_late_position.album.release_date = None;
    let result = fixture
        .discover(
            vec![
                undated_early_position,
                dated_candidate("103", "Live EP", "EP", "2021-06"),
                undated_late_position,
                dated_candidate("100", "Debut", "ALBUM", "1999"),
                dated_candidate("102", "Anniversary", "ALBUM", "2020-06-15"),
                dated_candidate("101", "Comeback", "ALBUM", "2020-06"),
            ],
            "a-guest",
        )
        .await;

    assert_eq!(result["returned_count"], 6);
    assert_eq!(album_ids(&result), ["103", "102", "101", "100", "90", "89"]);
}

#[tokio::test]
async fn discovery_places_a_group_at_the_release_date_of_the_edition_it_shows() {
    let fixture = Fixture::new(test_database().await);
    let result = fixture
        .discover(
            vec![
                dated_candidate("99", "Reissue album", "ALBUM", "2005"),
                dated_candidate("102", "Later", "ALBUM", "2021"),
                dated_candidate("100", "Reissue album", "ALBUM", "2020"),
                dated_candidate("101", "Earlier", "ALBUM", "1990"),
            ],
            "a-guest",
        )
        .await;

    // The higher numeric id (100, dated 2020) wins the "Reissue album" group, so
    // the group sits at 2020 between its neighbors instead of at its first-seen 2005.
    assert_eq!(result["returned_count"], 4);
    assert_eq!(album_ids(&result), ["102", "100", "101"]);
}

#[tokio::test]
async fn discovery_excludes_only_matching_title_and_type_for_any_credited_artist() {
    for (catalog_type, expected_ids) in [
        ("ALBUM", ["101", "102", "103", "104"]),
        ("SINGLE", ["100", "102", "103", "104"]),
        ("EP", ["100", "101", "103", "104"]),
    ] {
        let fixture = Fixture::new(test_database().await);
        fixture
            .add(candidate("1", " \tSAME\u{00a0} \n title ", catalog_type))
            .await;
        let mut unrelated = candidate("2", "same title", "ALBUM");
        unrelated.artists = vec![TidalArtist {
            id: "unrelated".to_owned(),
            name: "Unrelated Artist".to_owned(),
            profile_image_url: None,
        }];
        fixture.add(unrelated).await;
        let candidates = vec![
            candidate("100", "Same title", "ALBUM"),
            candidate("101", "Same title", "SINGLE"),
            candidate("102", "Same title", "EP"),
            candidate("103", "Same title (Deluxe)", "ALBUM"),
            candidate("104", "Same-title", "ALBUM"),
            candidate("105", "same  title", catalog_type),
        ];
        for artist_id in ["z-primary", "a-guest"] {
            let result = fixture.discover(candidates.clone(), artist_id).await;
            assert_eq!(result["returned_count"], 6);
            assert_eq!(
                album_ids(&result),
                expected_ids,
                "Catalog type {catalog_type}"
            );
        }
    }
}

#[tokio::test]
async fn discovery_rechecks_catalog_membership_after_addition_and_deletion() {
    let fixture = Fixture::new(test_database().await);
    let candidates = vec![
        candidate("100", "same title", "ALBUM"),
        candidate("101", "same title", "SINGLE"),
    ];
    let before_addition = fixture.discover(candidates.clone(), "a-guest").await;
    assert_eq!(album_ids(&before_addition), ["100", "101"]);

    for catalog_id in ["1", "2"] {
        fixture
            .add(candidate(catalog_id, " SAME  TITLE ", "SINGLE"))
            .await;
    }
    let owned = fixture.discover(candidates.clone(), "a-guest").await;
    assert_eq!(album_ids(&owned), ["100"]);

    for catalog_id in ["1", "2"] {
        let (status, _) = request(
            fixture.app(Vec::new()),
            "DELETE",
            &format!("/albums/{catalog_id}"),
            r#"{"delete_files":false}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let result = fixture.discover(candidates.clone(), "a-guest").await;
        let expected_ids = if catalog_id == "1" {
            vec!["100"]
        } else {
            vec!["100", "101"]
        };
        assert_eq!(album_ids(&result), expected_ids);
    }
}

#[tokio::test]
async fn discovery_does_not_guess_a_missing_catalog_release_type() {
    let fixture = Fixture::new(test_database().await);
    fixture.add(candidate("1", "Same title", "ALBUM")).await;
    album::ActiveModel {
        id: Set("1".to_owned()),
        album_type: Set(None),
        ..Default::default()
    }
    .update(&fixture.state.db)
    .await
    .unwrap();
    let result = fixture
        .discover(
            vec![
                candidate("100", "Same title", "ALBUM"),
                candidate("101", "Same title", "SINGLE"),
                candidate("102", "Same title", "EP"),
            ],
            "a-guest",
        )
        .await;
    assert_eq!(album_ids(&result), ["100", "101", "102"]);
}

fn ranked_candidate(id: &str, explicit: Option<bool>, media_tags: &[&str]) -> ScanAlbum {
    let mut record = candidate(id, "Same title", "ALBUM");
    record.album.explicit = explicit;
    record.album.media_tags = Some(media_tags.iter().map(|tag| (*tag).to_owned()).collect());
    record
}

async fn assert_winner(fixture: &Fixture, lower_ranked: ScanAlbum, higher_ranked: ScanAlbum) {
    let expected_album_id = higher_ranked.album.id.clone();
    for candidate_order in [
        vec![lower_ranked.clone(), higher_ranked.clone()],
        vec![higher_ranked, lower_ranked],
    ] {
        let result = fixture.discover(candidate_order, "a-guest").await;
        assert_eq!(result["returned_count"], 2);
        assert_eq!(album_ids(&result), [expected_album_id.as_str()]);
    }
}

#[tokio::test]
async fn discovery_ranks_explicitness_before_quality_before_numeric_id() {
    let fixture = Fixture::new(test_database().await);
    let explicitness_by_preference = [Some(false), None, Some(true)];
    let media_tags_by_quality: &[&[&str]] = &[
        &[],
        &["UNRECOGNIZED"],
        &["DOLBY_ATMOS"],
        &["LOSSLESS"],
        &["HIRES_LOSSLESS"],
    ];
    for (explicitness_rank, less_preferred_explicitness) in
        explicitness_by_preference.iter().enumerate()
    {
        for preferred_explicitness in &explicitness_by_preference[explicitness_rank + 1..] {
            for less_preferred_tags in media_tags_by_quality {
                for preferred_tags in media_tags_by_quality {
                    assert_winner(
                        &fixture,
                        ranked_candidate("100", *less_preferred_explicitness, less_preferred_tags),
                        ranked_candidate("99", *preferred_explicitness, preferred_tags),
                    )
                    .await;
                }
            }
        }
    }
    // Exclude duplicate unknown-quality cases from comparisons between different tiers.
    let distinct_qualities = &media_tags_by_quality[1..];
    for explicit in explicitness_by_preference {
        for (quality_rank, lower_quality_tags) in distinct_qualities.iter().enumerate() {
            for higher_quality_tags in &distinct_qualities[quality_rank + 1..] {
                assert_winner(
                    &fixture,
                    ranked_candidate("100", explicit, lower_quality_tags),
                    ranked_candidate("99", explicit, higher_quality_tags),
                )
                .await;
            }
        }
        for matching_quality_tags in media_tags_by_quality {
            assert_winner(
                &fixture,
                ranked_candidate("99", explicit, matching_quality_tags),
                ranked_candidate("100", explicit, matching_quality_tags),
            )
            .await;
        }
    }
    for lossless_tag in ["LOSSLESS", "HIRES_LOSSLESS"] {
        assert_winner(
            &fixture,
            ranked_candidate("99", None, &[lossless_tag, "DOLBY_ATMOS"]),
            ranked_candidate("100", None, &[lossless_tag]),
        )
        .await;
    }
    let mut unknown_quality = ranked_candidate("100", None, &[]);
    unknown_quality.album.media_tags = None;
    assert_winner(
        &fixture,
        unknown_quality,
        ranked_candidate("99", None, &["DOLBY_ATMOS"]),
    )
    .await;
}

#[tokio::test]
async fn discovery_returns_empty_results_when_all_title_and_type_groups_are_owned() {
    let fixture = Fixture::new(test_database().await);
    fixture.add(candidate("1", "Same title", "ALBUM")).await;
    let result = fixture
        .discover(
            vec![
                ranked_candidate("99", Some(false), &[]),
                ranked_candidate("100", Some(true), &["HIRES_LOSSLESS"]),
            ],
            "a-guest",
        )
        .await;
    assert_eq!(result["returned_count"], 2);
    assert!(album_ids(&result).is_empty());
}

#[tokio::test]
async fn discovery_catalog_errors_fail_instead_of_showing_unchecked_results() {
    let fixture = Fixture::new(Database::connect("sqlite::memory:").await.unwrap());
    let (status, _) = request(
        fixture.app(vec![candidate("100", "Same title", "ALBUM")]),
        "GET",
        "/tidal/artists/a-guest",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn discovery_rejects_invalid_numeric_ids_without_returning_partial_results() {
    let fixture = Fixture::new(test_database().await);
    for invalid_id in ["not-numeric", "18446744073709551616"] {
        let (status, _) = request(
            fixture.app(vec![
                candidate("100", "Valid title", "ALBUM"),
                candidate(invalid_id, "Different title", "ALBUM"),
            ]),
            "GET",
            "/tidal/artists/a-guest",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }
}

#[tokio::test]
async fn discovery_propagates_discography_errors() {
    let fixture = Fixture::new(test_database().await);
    let mut failed_record = candidate("100", "Same title", "ALBUM");
    failed_record.failure = Some("discography");
    let (status, _) = request(
        fixture.app(vec![failed_record]),
        "GET",
        "/tidal/artists/a-guest",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
