use std::time::Duration;

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
