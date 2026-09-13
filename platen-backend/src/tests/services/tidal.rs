use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Router, http::Uri, routing::get};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use url::Url;

use crate::services::rate_limit::RateLimit;

use super::tidal_response::{
    AlbumSearch, AlbumSearchIncluded, AlbumSearchIncludedAttributes, AlbumWithArtistsDocument,
    ArtistAlbumsRelationshipDocument, ArtistSingleResource, ArtworkRelationship,
    ArtworkRelationshipData, ArtworkResource,
};
use super::{
    artwork_resources, resolve_album_search, select_artwork_url, select_profile_image_url,
};

#[test]
fn album_track_count_reads_total_items_from_metadata() {
    let document: super::tidal_response::AlbumTrackCountDocument =
        serde_json::from_value(serde_json::json!({
            "data": {
                "id": "123",
                "type": "albums",
                "attributes": {"numberOfItems": 30}
            }
        }))
        .unwrap();
    assert_eq!(document.data.unwrap().attributes.number_of_items, 30);
}

#[test]
fn album_track_count_rejects_missing_or_invalid_counts() {
    for attributes in [
        serde_json::json!({}),
        serde_json::json!({"numberOfItems": null}),
        serde_json::json!({"numberOfItems": -1}),
        serde_json::json!({"numberOfItems": "12"}),
    ] {
        assert!(
            serde_json::from_value::<super::tidal_response::AlbumTrackCountDocument>(
                serde_json::json!({"data": {"attributes": attributes}})
            )
            .is_err()
        );
    }
}

#[test]
fn discovery_contract_preserves_metadata_and_selects_the_highest_known_quality() {
    use crate::routes::tidal::dto;
    use serde_json::{Value, json};

    for (explicit, tags, quality) in [
        (Some(true), Some(vec!["LOSSLESS"]), Some("LOSSLESS")),
        (
            Some(false),
            Some(vec!["HIRES_LOSSLESS"]),
            Some("HIRES_LOSSLESS"),
        ),
        (
            None,
            Some(vec!["LOSSLESS", "HIRES_LOSSLESS"]),
            Some("HIRES_LOSSLESS"),
        ),
        (
            Some(true),
            Some(vec!["HIRES_LOSSLESS", "LOSSLESS", "FUTURE"]),
            Some("HIRES_LOSSLESS"),
        ),
        (
            Some(false),
            Some(vec!["FUTURE", "LOSSLESS"]),
            Some("LOSSLESS"),
        ),
        (None, Some(vec!["DOLBY_ATMOS"]), Some("DOLBY_ATMOS")),
        (
            None,
            Some(vec!["DOLBY_ATMOS", "LOSSLESS"]),
            Some("LOSSLESS + DOLBY_ATMOS"),
        ),
        (
            None,
            Some(vec!["LOSSLESS", "DOLBY_ATMOS"]),
            Some("LOSSLESS + DOLBY_ATMOS"),
        ),
        (
            None,
            Some(vec!["HIRES_LOSSLESS", "DOLBY_ATMOS"]),
            Some("HIRES_LOSSLESS + DOLBY_ATMOS"),
        ),
        (
            None,
            Some(vec!["DOLBY_ATMOS", "LOSSLESS", "HIRES_LOSSLESS"]),
            Some("HIRES_LOSSLESS + DOLBY_ATMOS"),
        ),
        (
            None,
            Some(vec!["HIRES_LOSSLESS", "LOSSLESS", "DOLBY_ATMOS", "FUTURE"]),
            Some("HIRES_LOSSLESS + DOLBY_ATMOS"),
        ),
        (
            None,
            Some(vec!["FUTURE", "DOLBY_ATMOS", "DOLBY_ATMOS"]),
            Some("DOLBY_ATMOS"),
        ),
        (None, Some(vec!["FUTURE"]), None),
        (None, Some(vec![]), None),
        (None, None, None),
    ] {
        let mut attributes = json!({"title": "Same title", "type": "ALBUM", "popularity": 0.5});
        if let Some(explicit) = explicit {
            attributes["explicit"] = json!(explicit);
        }
        if let Some(tags) = &tags {
            attributes["mediaTags"] = json!(tags);
        }
        let included = json!([
            {"id": "edition-2", "type": "albums", "attributes": attributes},
            {"id": "edition-1", "type": "albums", "attributes": attributes}
        ]);
        let search: AlbumSearch = serde_json::from_value(json!({
            "data": [{"relationships": {"albums": {"data": [{"id": "edition-2"}, {"id": "edition-1"}]}}}],
            "included": included
        })).unwrap();
        let search_albums: Vec<Value> = resolve_album_search(&search)
            .unwrap()
            .into_iter()
            .map(|album| serde_json::to_value(dto::TidalAlbumSearchHit::from(album)).unwrap())
            .collect();
        let releases: ArtistAlbumsRelationshipDocument =
            serde_json::from_value(json!({"included": included})).unwrap();
        let release_albums: Vec<Value> = releases
            .included
            .into_iter()
            .map(|resource| {
                let AlbumSearchIncluded::Album { id, attributes, .. } = resource else {
                    panic!("expected album");
                };
                serde_json::to_value(dto::TidalAlbum::from(super::TidalAlbum::from(
                    id, attributes, None,
                )))
                .unwrap()
            })
            .collect();
        for albums in [search_albums, release_albums] {
            assert_eq!(
                albums
                    .iter()
                    .map(|album| album["id"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["edition-2", "edition-1"]
            );
            for album in albums {
                assert_eq!(album["explicit"], json!(explicit));
                assert_eq!(album["media_tags"], json!(tags));
                assert_eq!(album["available_quality"], json!(quality));
            }
        }
    }
}

#[test]
fn unknown_media_tags_are_logged_even_after_the_highest_quality_is_found() {
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct LogWriter(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = LogWriter(output.clone());
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let tags = [
            "HIRES_LOSSLESS".to_owned(),
            "DOLBY_ATMOS".to_owned(),
            "FUTURE_CODEC".to_owned(),
            "OTHER_CODEC".to_owned(),
        ];
        assert_eq!(
            super::available_quality(Some(&tags)),
            Some("HIRES_LOSSLESS + DOLBY_ATMOS")
        );
    });
    let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("FUTURE_CODEC"));
    assert!(logs.contains("OTHER_CODEC"));
    assert!(logs.contains("WARN"));
    assert!(!logs.contains("DOLBY_ATMOS"));
    assert!(!logs.contains("HIRES_LOSSLESS"));
}

#[test]
fn catalog_requests_include_the_configured_country() {
    for country_code in ["NL", "US"] {
        let tidal = super::Tidal::new(
            String::new(),
            String::new(),
            country_code.to_owned(),
            RateLimit::new(Duration::ZERO),
        );
        for path in [
            "/searchResults",
            "/artists/123?include=profileArt",
            "/artists/123/relationships/albums?include=albums,albums.coverArt",
            "/albums/456",
            "/albums/456?include=coverArt",
            "/albums/456?include=artists,artists.profileArt",
        ] {
            let url = format!("{}{path}", super::TIDAL_BASE_URL);
            let original_url = url::Url::parse(&url).unwrap();
            let request = tidal.catalog_request(&url).unwrap().build().unwrap();
            let query: Vec<_> = request.url().query_pairs().collect();
            assert_eq!(request.method(), reqwest::Method::GET);
            assert_eq!(request.url().path(), original_url.path());
            assert_eq!(
                query
                    .iter()
                    .filter(|(name, _)| name == "countryCode")
                    .count(),
                1
            );
            assert!(
                query
                    .iter()
                    .any(|(name, value)| name == "countryCode" && value == country_code)
            );
            for pair in original_url.query_pairs() {
                assert!(query.contains(&pair));
            }
        }
    }
}

#[test]
fn pagination_preserves_the_cursor_and_enforces_one_configured_country() {
    let tidal = super::Tidal::new(
        String::new(),
        String::new(),
        "NL".to_owned(),
        RateLimit::new(Duration::ZERO),
    );
    for country_query in ["", "&countryCode=NL", "&countryCode=US&countryCode=GB"] {
        let url = format!(
            "{}/artists/123/relationships/albums?page%5Bcursor%5D=a%2Bb%2F%3D&include=albums,albums.coverArt{country_query}",
            super::TIDAL_BASE_URL
        );
        let request = tidal.catalog_request(&url).unwrap().build().unwrap();
        let query: Vec<_> = request.url().query_pairs().collect();
        assert_eq!(query.len(), 3);
        assert!(
            query
                .iter()
                .any(|(name, value)| name == "page[cursor]" && value == "a+b/=")
        );
        assert!(
            query
                .iter()
                .any(|(name, value)| name == "include" && value == "albums,albums.coverArt")
        );
        assert!(
            query
                .iter()
                .any(|(name, value)| name == "countryCode" && value == "NL")
        );
    }
}

fn artist_albums_page(page: usize, has_next: bool) -> Value {
    let next = has_next.then(|| {
        format!(
            "/artists/123/relationships/albums?page%5Bcursor%5D={}&include=albums,albums.coverArt&countryCode=US&countryCode=GB",
            page + 1
        )
    });
    json!({
        "data": [{"id": page.to_string(), "type": "albums"}],
        "included": [{
            "id": page.to_string(),
            "type": "albums",
            "attributes": {"title": format!("Album {page}"), "type": "ALBUM", "popularity": 0.5}
        }],
        "links": {"next": next}
    })
}

async fn fetch_artist_album_pages(
    pages: Vec<(StatusCode, String)>,
) -> (Result<Vec<super::TidalAlbum>, super::TidalError>, Vec<Url>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = Arc::clone(&requests);
    let pages = Arc::new(pages);
    let app = Router::new().route(
        "/v2/artists/123/relationships/albums",
        get(move |uri: Uri| {
            let requests = Arc::clone(&captured_requests);
            let pages = Arc::clone(&pages);
            async move {
                let url = Url::parse(&format!("http://localhost{uri}")).unwrap();
                let page = url
                    .query_pairs()
                    .find(|(name, _)| name == "page[cursor]")
                    .map(|(_, cursor)| cursor.parse::<usize>().unwrap())
                    .unwrap_or(1);
                requests.lock().unwrap().push(url);
                pages
                    .get(page - 1)
                    .cloned()
                    .unwrap_or((StatusCode::NOT_FOUND, String::new()))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v2", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let tidal = super::Tidal {
        client: reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
        auth: Arc::new(tokio::sync::Mutex::new(super::TidalAuth {
            token: Some("test-token".to_owned()),
            expires_at: chrono::Utc::now() + Duration::from_secs(60),
        })),
        rate_limit: RateLimit::new(Duration::ZERO),
        client_id: String::new(),
        client_secret: String::new(),
        country_code: "NL".to_owned(),
    };

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tidal.get_artist_albums_from("123", &base_url),
    )
    .await;
    server.abort();
    let requests = requests.lock().unwrap().clone();
    (result.expect("discography request timed out"), requests)
}

async fn assert_artist_albums_total_deadline(retry_after: bool) {
    use axum::response::IntoResponse;
    use tokio::sync::{mpsc, oneshot};

    // Real socket I/O must finish before Tokio advances the paused clock.
    let clock_guard = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    let (request_sender, mut requests) = mpsc::unbounded_channel();
    let app = Router::new().route(
        "/v2/artists/123/relationships/albums",
        get(move || {
            let request_sender = request_sender.clone();
            async move {
                let (response_sender, response_receiver) =
                    oneshot::channel::<axum::response::Response>();
                request_sender.send(response_sender).unwrap();
                response_receiver.await.unwrap()
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v2", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let tidal = super::Tidal {
        // The ordinary fixture's two-second request timeout would mask this deadline.
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
        auth: Arc::new(tokio::sync::Mutex::new(super::TidalAuth {
            token: Some("test-token".to_owned()),
            expires_at: chrono::Utc::now() + Duration::from_secs(3600),
        })),
        rate_limit: RateLimit::new(Duration::ZERO),
        client_id: String::new(),
        client_secret: String::new(),
        country_code: "NL".to_owned(),
    };
    let operation = tidal.get_artist_albums_from("123", &base_url);
    tokio::pin!(operation);

    let first_response = tokio::select! {
        result = &mut operation => panic!("discography finished before page one: {result:?}"),
        response = requests.recv() => response.unwrap(),
    };
    tokio::time::advance(Duration::from_secs(400)).await;
    first_response
        .send(artist_albums_page(1, true).to_string().into_response())
        .unwrap();
    let second_response = tokio::select! {
        result = &mut operation => panic!("discography finished before page two: {result:?}"),
        response = requests.recv() => response.unwrap(),
    };
    if retry_after {
        second_response
            .send((StatusCode::TOO_MANY_REQUESTS, [("retry-after", "1200")]).into_response())
            .unwrap();
    }
    // The second page starts with only 200 seconds left, not a new ten-minute budget.
    tokio::time::advance(Duration::from_secs(199)).await;
    assert!(futures_util::poll!(&mut operation).is_pending());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        futures_util::poll!(&mut operation),
        std::task::Poll::Ready(Err(super::TidalError::ArtistAlbumsTimeout))
    ));
    assert!(requests.try_recv().is_err());
    server.abort();
    clock_guard.abort();
}

#[tokio::test(start_paused = true)]
async fn artist_albums_timeout_discards_a_successful_page_when_the_next_stalls() {
    assert_artist_albums_total_deadline(false).await;
}

#[tokio::test(start_paused = true)]
async fn artist_albums_timeout_includes_retry_after_on_a_later_page() {
    assert_artist_albums_total_deadline(true).await;
}

#[tokio::test(start_paused = true)]
async fn artist_albums_timeout_includes_waiting_for_authentication() {
    let tidal = super::Tidal::new(
        String::new(),
        String::new(),
        "NL".to_owned(),
        RateLimit::new(Duration::ZERO),
    );
    let _auth_lock = tidal.auth.lock().await;
    let operation = tidal.get_artist_albums("123");
    tokio::pin!(operation);

    assert!(futures_util::poll!(&mut operation).is_pending());
    tokio::time::advance(Duration::from_secs(599)).await;
    assert!(futures_util::poll!(&mut operation).is_pending());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        futures_util::poll!(&mut operation),
        std::task::Poll::Ready(Err(super::TidalError::ArtistAlbumsTimeout))
    ));
}

#[tokio::test]
async fn artist_albums_rejects_remaining_pages_at_the_limit() {
    let pages = (1..=51)
        .map(|page| {
            (
                StatusCode::OK,
                artist_albums_page(page, page < 51).to_string(),
            )
        })
        .collect();
    let (result, requests) = fetch_artist_album_pages(pages).await;

    assert_eq!(requests.len(), 50);
    assert!(matches!(result, Err(super::TidalError::UnexpectedResponse)));
}

#[tokio::test]
async fn artist_albums_accepts_completion_exactly_at_the_limit() {
    let pages = (1..=50)
        .map(|page| {
            (
                StatusCode::OK,
                artist_albums_page(page, page < 50).to_string(),
            )
        })
        .collect();
    let (result, requests) = fetch_artist_album_pages(pages).await;

    assert_eq!(requests.len(), 50);
    assert_eq!(
        result
            .unwrap()
            .into_iter()
            .map(|album| album.id)
            .collect::<Vec<_>>(),
        (1..=50).map(|page| page.to_string()).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn artist_albums_collects_pages_and_preserves_the_configured_territory() {
    let pages = (1..=3)
        .map(|page| {
            (
                StatusCode::OK,
                artist_albums_page(page, page < 3).to_string(),
            )
        })
        .collect();
    let (result, requests) = fetch_artist_album_pages(pages).await;
    let albums = result.unwrap();

    assert_eq!(requests.len(), 3);
    assert_eq!(albums.len(), 3);
    for (index, album) in albums.iter().enumerate() {
        assert_eq!(album.id, (index + 1).to_string());
        assert_eq!(album.title, format!("Album {}", index + 1));
        assert_eq!(album.r#type, "ALBUM");
        assert_eq!(album.popularity, 0.5);
        assert!(album.cover_url.is_none());
        assert!(album.release_date.is_none());
        assert!(album.explicit.is_none());
        assert!(album.media_tags.is_none());
    }
    for request in requests {
        assert_eq!(
            request
                .query_pairs()
                .filter(|(name, _)| name == "countryCode")
                .map(|(_, value)| value.into_owned())
                .collect::<Vec<_>>(),
            ["NL"]
        );
        assert!(
            request
                .query_pairs()
                .any(|(name, value)| name == "include" && value == "albums,albums.coverArt")
        );
    }
}

#[tokio::test]
async fn artist_albums_accepts_an_empty_discography() {
    let (result, requests) = fetch_artist_album_pages(vec![(
        StatusCode::OK,
        json!({"data": [], "included": [], "links": {"next": null}}).to_string(),
    )])
    .await;

    assert_eq!(requests.len(), 1);
    assert!(result.unwrap().is_empty());
}

#[tokio::test]
async fn artist_albums_propagates_http_errors_after_a_successful_page() {
    let (result, requests) = fetch_artist_album_pages(vec![
        (StatusCode::OK, artist_albums_page(1, true).to_string()),
        (StatusCode::SERVICE_UNAVAILABLE, "Unavailable".to_owned()),
    ])
    .await;

    assert_eq!(requests.len(), 2);
    assert!(matches!(result, Err(super::TidalError::UnexpectedResponse)));
}

#[tokio::test]
async fn artist_albums_propagates_json_errors_after_a_successful_page() {
    let (result, requests) = fetch_artist_album_pages(vec![
        (StatusCode::OK, artist_albums_page(1, true).to_string()),
        (StatusCode::OK, "{".to_owned()),
    ])
    .await;

    assert_eq!(requests.len(), 2);
    assert!(matches!(result, Err(super::TidalError::Reqwest(error)) if error.is_decode()));
}

#[test]
fn search_parameters_are_preserved_alongside_the_country() {
    let tidal = super::Tidal::new(
        String::new(),
        String::new(),
        "NL".to_owned(),
        RateLimit::new(Duration::ZERO),
    );
    let request = tidal
        .catalog_request(&format!("{}/searchResults", super::TIDAL_BASE_URL))
        .unwrap()
        .query(&[
            ("filter[query]", "Artist & Album"),
            ("include", "albums.artists"),
        ])
        .build()
        .unwrap();
    let query: Vec<_> = request.url().query_pairs().collect();
    assert_eq!(query.len(), 3);
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "countryCode" && value == "NL")
    );
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "filter[query]" && value == "Artist & Album")
    );
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "include" && value == "albums.artists")
    );
}

#[test]
fn resolves_album_artists_in_relationship_order() {
    let response: AlbumSearch = serde_json::from_value(serde_json::json!({
        "data": [{
            "relationships": {
                "albums": {
                    "data": [{"id": "album-1", "type": "albums"}]
                }
            }
        }],
        "included": [
            {
                "type": "albums",
                "id": "album-1",
                "attributes": {
                    "title": "Shared Billing",
                    "releaseDate": "2026-09-02",
                    "popularity": 0.5,
                    "type": "ALBUM"
                },
                "relationships": {
                    "artists": {
                        "data": [
                            {"id": "primary", "type": "artists"},
                            {"id": "featured", "type": "artists"}
                        ]
                    }
                }
            },
            {
                "type": "artists",
                "id": "featured",
                "attributes": {"name": "Featured Artist"}
            },
            {
                "type": "artists",
                "id": "primary",
                "attributes": {"name": "Primary Artist"}
            }
        ]
    }))
    .unwrap();

    let albums = resolve_album_search(&response).unwrap();
    let artist_ids: Vec<_> = albums[0]
        .artists
        .iter()
        .map(|artist| artist.id.as_str())
        .collect();

    assert_eq!(artist_ids, ["primary", "featured"]);
}

/// Regression: `#[serde(rename = "camelCase")]` renames the type, not the
/// fields, so Tidal's `releaseDate` silently deserialized to `None`.
/// `rename_all` is the fix; this test pins the camelCase field names.
#[test]
fn deserializes_camel_case_attributes() {
    let json = r#"
        {
          "title": "Michelle (Take 1)",
          "releaseDate": "2026-07-29",
          "popularity": 0.7160302460678571,
          "type": "SINGLE"
        }
    "#;

    let attr: AlbumSearchIncludedAttributes = serde_json::from_str(json).unwrap();

    assert_eq!(attr.title, "Michelle (Take 1)");
    assert_eq!(attr.release_date.as_deref(), Some("2026-07-29"));
    assert!((attr.popularity - 0.7160302460678571).abs() < f64::EPSILON);
    assert_eq!(attr.r#type, "SINGLE");
}

/// Regression for the same snake_case bug as above, against the exact
/// shape `get_artist_albums` deserializes.
#[test]
fn deserializes_artist_albums_relationship_document() {
    let json = r#"
        {
          "data": [
            {"id": "546629982", "type": "albums"}
          ],
          "included": [
            {
              "id": "546629982",
              "type": "albums",
              "attributes": {
                "title": "Michelle (Take 1)",
                "releaseDate": "2026-07-29",
                "popularity": 0.7160302460678571,
                "type": "SINGLE"
              }
            }
          ],
          "links": null
        }
    "#;

    let doc: ArtistAlbumsRelationshipDocument = serde_json::from_str(json).unwrap();

    assert_eq!(doc.included.len(), 1);
    let AlbumSearchIncluded::Album { id, attributes, .. } = &doc.included[0] else {
        panic!("expected an album resource");
    };
    assert_eq!(id, "546629982");
    assert_eq!(attributes.release_date.as_deref(), Some("2026-07-29"));
    assert!(doc.links.is_none());
}

#[test]
fn deserializes_album_artists_with_profile_artwork() {
    let doc: AlbumWithArtistsDocument = serde_json::from_value(serde_json::json!({
        "data": {
            "type": "albums",
            "id": "album-1",
            "attributes": {
                "title": "Duality",
                "duration": "PT30M",
                "explicit": false,
                "popularity": 0.0,
                "type": "ALBUM"
            },
            "relationships": {
                "artists": {
                    "data": [{"id": "artist-1", "type": "artists"}]
                }
            }
        },
        "included": [
            {
                "type": "artists",
                "id": "artist-1",
                "attributes": {"name": "BLCKK"},
                "relationships": {
                    "profileArt": {"data": [{"id": "portrait", "type": "artworks"}]}
                }
            },
            {
                "type": "artworks",
                "id": "portrait",
                "attributes": {
                    "mediaType": "IMAGE",
                    "files": [{
                        "href": "https://cdn.example/portrait",
                        "width": 640,
                        "height": 640
                    }]
                }
            }
        ]
    }))
    .unwrap();

    let artworks: Vec<_> = artwork_resources(&doc.included).collect();
    let AlbumSearchIncluded::Artist {
        id,
        attributes,
        relationships,
    } = doc
        .included
        .iter()
        .find(|included| matches!(included, AlbumSearchIncluded::Artist { .. }))
        .unwrap()
    else {
        panic!("expected an artist resource");
    };

    assert_eq!(id, "artist-1");
    assert_eq!(attributes.name, "BLCKK");
    assert_eq!(
        select_profile_image_url(relationships.as_ref(), &artworks).as_deref(),
        Some("https://cdn.example/portrait")
    );
}

#[test]
fn selects_profile_image_from_artist_metadata() {
    let doc: ArtistSingleResource = serde_json::from_value(serde_json::json!({
        "data": {
            "type": "artists",
            "id": "artist-1",
            "attributes": {"name": "BLCKK"},
            "relationships": {
                "profileArt": {"data": [{"id": "portrait", "type": "artworks"}]}
            }
        },
        "included": [{
            "type": "artworks",
            "id": "portrait",
            "attributes": {
                "mediaType": "IMAGE",
                "files": [
                    {"href": "https://cdn.example/portrait-large", "width": 1200, "height": 1200},
                    {"href": "https://cdn.example/portrait", "width": 640, "height": 640}
                ]
            }
        }]
    }))
    .unwrap();
    let artist = doc.data.unwrap();
    let artworks: Vec<_> = artwork_resources(&doc.included).collect();

    assert_eq!(
        select_profile_image_url(artist.relationships.as_ref(), &artworks).as_deref(),
        Some("https://cdn.example/portrait")
    );
}

#[test]
fn selects_the_first_image_resource_with_the_best_square_file() {
    let included: Vec<AlbumSearchIncluded> = serde_json::from_value(serde_json::json!([
        {
            "type": "artworks",
            "id": "video",
            "attributes": {
                "mediaType": "VIDEO",
                "files": [{"href": "https://cdn.example/video", "width": 2000, "height": 2000}]
            }
        },
        {
            "type": "artworks",
            "id": "not-square",
            "attributes": {
                "mediaType": "IMAGE",
                "files": [{"href": "https://cdn.example/landscape", "width": 1200, "height": 800}]
            }
        },
        {
            "type": "artworks",
            "id": "cover",
            "attributes": {
                "mediaType": "IMAGE",
                "files": [
                    {"href": "http://cdn.example/http", "width": 1600, "height": 1600},
                    {"href": "/relative", "width": 1600, "height": 1600},
                    {"href": "not a url", "width": 1600, "height": 1600},
                    {"href": "https://cdn.example/large", "width": 1200, "height": 1200},
                    {"href": "https://cdn.example/small", "width": 640, "height": 640},
                    {"href": "https://cdn.example/tiny", "width": 320, "height": 320}
                ]
            }
        }
    ]))
    .unwrap();
    let relationship = ArtworkRelationship {
        data: Some(
            [
                ArtworkRelationshipData { id: "video".into() },
                ArtworkRelationshipData {
                    id: "not-square".into(),
                },
                ArtworkRelationshipData { id: "cover".into() },
            ]
            .into(),
        ),
    };

    let artworks: Vec<_> = artwork_resources(&included).collect();
    let cover = select_artwork_url(Some(&relationship), &artworks);

    assert_eq!(cover.as_deref(), Some("https://cdn.example/small"));
}

#[test]
fn selects_the_first_valid_artwork_resource_in_relationship_order() {
    let included: Vec<AlbumSearchIncluded> = serde_json::from_value(serde_json::json!([
        {
            "type": "artworks",
            "id": "first",
            "attributes": {
                "mediaType": "IMAGE",
                "files": [{"href": "https://cdn.example/first", "width": 1200, "height": 1200}]
            }
        },
        {
            "type": "artworks",
            "id": "second",
            "attributes": {
                "mediaType": "IMAGE",
                "files": [{"href": "https://cdn.example/second", "width": 640, "height": 640}]
            }
        }
    ]))
    .unwrap();
    let relationship = ArtworkRelationship {
        data: Some(
            [
                ArtworkRelationshipData { id: "first".into() },
                ArtworkRelationshipData {
                    id: "second".into(),
                },
            ]
            .into(),
        ),
    };

    let artworks: Vec<_> = artwork_resources(&included).collect();
    assert_eq!(
        select_artwork_url(Some(&relationship), &artworks).as_deref(),
        Some("https://cdn.example/first")
    );
}

#[test]
fn does_not_combine_dimensions_from_different_metadata_pairs() {
    let resource: ArtworkResource = serde_json::from_value(serde_json::json!({
        "id": "cover",
        "attributes": {
            "mediaType": "IMAGE",
            "files": [{
                "href": "https://cdn.example/mixed",
                "width": 640,
                "meta": {"width": 1200, "height": 640}
            }]
        }
    }))
    .unwrap();
    let relationship = ArtworkRelationship {
        data: Some(vec![ArtworkRelationshipData { id: "cover".into() }]),
    };

    let artworks = vec![&resource];
    assert_eq!(select_artwork_url(Some(&relationship), &artworks), None);
}

#[test]
fn uses_largest_square_when_no_square_file_reaches_the_threshold() {
    let resource: ArtworkResource = serde_json::from_value(serde_json::json!({
        "id": "cover",
        "attributes": {
            "mediaType": "IMAGE",
            "files": [
                {"href": "https://cdn.example/medium", "meta": {"width": 500, "height": 500}},
                {"href": "https://cdn.example/large", "width": 600, "height": 600},
                {"href": "https://cdn.example/wide", "width": 1000, "height": 800}
            ]
        }
    }))
    .unwrap();
    let relationship = ArtworkRelationship {
        data: Some(vec![ArtworkRelationshipData { id: "cover".into() }]),
    };

    let artworks = vec![&resource];
    assert_eq!(
        select_artwork_url(Some(&relationship), &artworks).as_deref(),
        Some("https://cdn.example/large")
    );
}
