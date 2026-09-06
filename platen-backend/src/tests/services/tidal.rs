use super::tidal_response::{
    AlbumSearch, AlbumSearchIncluded, AlbumSearchIncludedAttributes, AlbumWithArtistsDocument,
    ArtistAlbumsRelationshipDocument, ArtistSingleResource, ArtworkRelationship,
    ArtworkRelationshipData, ArtworkResource,
};
use super::{
    artwork_resources, resolve_album_search, select_artwork_url, select_profile_image_url,
};

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
