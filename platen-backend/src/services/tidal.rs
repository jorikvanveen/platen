use chrono::Utc;
use reqwest::{RequestBuilder, StatusCode};
use std::{sync::Arc, time::Duration};
use thiserror::Error;
use tokio::{sync::Mutex, time::sleep};
use url::Url;

use tidal_response::{
    AlbumSearchIncluded, AlbumSearchIncludedAttributes, AlbumSearchRelationshipsAlbumsData,
    AlbumSingleResource, AlbumWithArtistsDocument, ArtistAlbumsRelationshipDocument,
    ArtistSearchDocument,
};

static TIDAL_BASE_URL: &str = "https://openapi.tidal.com/v2";

#[derive(Error, Debug)]
pub enum TidalError {
    #[error("Tidal API sent back an unexpected response")]
    UnexpectedResponse,

    #[error("Error sending request to tidal API: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Failed to authenticate with tidal API: {0}: {1}")]
    AuthenticationFailed(StatusCode, String),
}

struct TidalAuth {
    token: Option<String>,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct Tidal {
    client: reqwest::Client,
    auth: Arc<Mutex<TidalAuth>>,
    client_id: String,
    client_secret: String,
}

impl Tidal {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client: reqwest::ClientBuilder::new().build().expect(
                "reqwest client build only fails on TLS misconfiguration, which is static here",
            ),
            auth: Arc::new(Mutex::new(TidalAuth {
                token: None,
                expires_at: Default::default(),
            })),
            client_id,
            client_secret,
        }
    }

    pub async fn login(&self) -> Result<(), TidalError> {
        self.ensure_token().await?;
        tracing::info!("Logged in to tidal");
        Ok(())
    }

    async fn send_with_retry(
        &self,
        builder: RequestBuilder,
    ) -> Result<reqwest::Response, TidalError> {
        const MAX_RETRIES: u32 = 3;
        let mut retries: u32 = 0;
        let mut retried_auth = false;

        loop {
            let token = self.ensure_token().await?;
            let req = builder
                .try_clone()
                .ok_or(TidalError::UnexpectedResponse)?
                .bearer_auth(token);

            match req.send().await {
                Ok(res)
                    if res.status() == StatusCode::TOO_MANY_REQUESTS && retries < MAX_RETRIES =>
                {
                    let wait_secs = res
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or_else(|| 1 << retries);

                    sleep(Duration::from_secs(wait_secs)).await;
                    retries += 1;
                }
                Ok(res) if res.status() == StatusCode::UNAUTHORIZED && !retried_auth => {
                    self.auth.lock().await.token = None;
                    retried_auth = true;
                }
                Ok(res) => return Ok(res),
                Err(_) if retries < MAX_RETRIES => {
                    sleep(Duration::from_secs(1 << retries)).await;
                    retries += 1;
                }
                Err(e) => return Err(TidalError::Reqwest(e)),
            }
        }
    }

    async fn get_oauth_token(&self) -> Result<String, TidalError> {
        let response = self
            .client
            .post("https://auth.tidal.com/v1/oauth2/token")
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("grant_type=client_credentials")
            .send()
            .await?;

        if response.status() != StatusCode::OK {
            return Err(TidalError::AuthenticationFailed(
                response.status(),
                response.text().await?,
            ));
        }

        let response: tidal_response::OauthToken = response
            .json()
            .await
            .map_err(|_| TidalError::UnexpectedResponse)?;

        let token = response.access_token;
        let mut auth = self.auth.lock().await;
        auth.token = Some(token.clone());
        auth.expires_at =
            chrono::Utc::now() + Duration::from_secs(response.expires_in.saturating_sub(60));

        tracing::info!("Authenticated with tidal");
        Ok(token)
    }

    async fn ensure_token(&self) -> Result<String, TidalError> {
        {
            let auth = self.auth.lock().await;
            let now = chrono::Utc::now();
            if let Some(token) = auth.token.clone().filter(|_| auth.expires_at > now) {
                return Ok(token);
            }
        }
        self.get_oauth_token().await
    }

    pub async fn find_album(
        &self,
        query: &str,
    ) -> Result<Vec<ResolvedTidalSearchedAlbum>, TidalError> {
        let url = format!("{TIDAL_BASE_URL}/searchResults");
        let request = self.client.get(url).query(&[
            ("filter[query]", query),
            (
                "include",
                "albums.artists,albums.artists.profileArt,albums.coverArt",
            ),
        ]);

        let resp = self.send_with_retry(request).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let resp: tidal_response::AlbumSearch = resp.json().await?;

        let relationships = &resp
            .data
            .first()
            .ok_or(TidalError::UnexpectedResponse)?
            .relationships
            .albums
            .data;
        let artworks: Vec<_> = artwork_resources(&resp.included).collect();

        relationships
            .iter()
            .map(|relationship| {
                let (album, artist_ids, cover_url) = resp
                    .included
                    .iter()
                    .find_map(|included| match included {
                        AlbumSearchIncluded::Album {
                            id,
                            attributes,
                            relationships,
                        } if id == &relationship.id => Some((
                            attributes,
                            relationships
                                .artists
                                .data
                                .iter()
                                .map(|artist| artist.id.clone())
                                .collect::<Vec<_>>(),
                            select_artwork_url(relationships.cover_art.as_ref(), &artworks),
                        )),
                        _ => None,
                    })
                    .ok_or(TidalError::UnexpectedResponse)?;

                let artists = resp
                    .included
                    .iter()
                    .filter_map(|included| match included {
                        AlbumSearchIncluded::Artist {
                            id,
                            attributes,
                            relationships,
                        } if artist_ids.iter().any(|artist_id| artist_id == id) => {
                            Some(TidalArtist {
                                id: id.clone(),
                                name: attributes.name.clone(),
                                profile_image_url: select_profile_image_url(
                                    relationships.as_ref(),
                                    &artworks,
                                ),
                            })
                        }
                        _ => None,
                    })
                    .collect();

                Ok(resolve_searched_album(
                    relationship,
                    album,
                    artists,
                    cover_url,
                ))
            })
            .collect()
    }

    pub async fn get_album(&self, id: &str) -> Result<TidalAlbum, TidalError> {
        let url = format!("{TIDAL_BASE_URL}/albums/{id}");
        let resp = self.send_with_retry(self.client.get(url)).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let doc: AlbumSingleResource = resp.json().await?;
        let resource = doc.data.ok_or(TidalError::UnexpectedResponse)?;
        Ok(TidalAlbum::from(resource.id, resource.attributes, None))
    }

    pub async fn get_album_cover(&self, id: &str) -> Result<Option<String>, TidalError> {
        let url = format!("{TIDAL_BASE_URL}/albums/{id}?include=coverArt");
        let resp = self.send_with_retry(self.client.get(url)).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let doc: AlbumSingleResource = resp.json().await?;
        let resource = doc.data.ok_or(TidalError::UnexpectedResponse)?;
        let artworks: Vec<_> = artwork_resources(&doc.included).collect();
        Ok(select_artwork_url(
            resource
                .relationships
                .as_ref()
                .and_then(|relationships| relationships.cover_art.as_ref()),
            &artworks,
        ))
    }

    /// Follows `links.next` cursor pages of
    /// `GET /artists/{id}/relationships/albums?include=albums,albums.coverArt`
    /// until exhausted or `MAX_PAGES` is hit. The plain
    /// `GET /artists/{id}?include=albums` endpoint exposes no paging parameter
    /// and returns only the first page,
    /// which is why this goes through the relationship endpoint instead.
    pub async fn get_artist_albums(&self, id: &str) -> Result<Vec<TidalAlbum>, TidalError> {
        const MAX_PAGES: usize = 50;
        let mut albums: Vec<TidalAlbum> = Vec::new();
        let mut next: Option<String> = None;

        for page in 0..MAX_PAGES {
            let url = match &next {
                Some(rel) => format!("{TIDAL_BASE_URL}{rel}"),
                None => {
                    format!(
                        "{TIDAL_BASE_URL}/artists/{id}/relationships/albums?include=albums,albums.coverArt"
                    )
                }
            };
            let resp = self.send_with_retry(self.client.get(url)).await?;
            if !resp.status().is_success() {
                tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
                return Err(TidalError::UnexpectedResponse);
            }

            let doc: ArtistAlbumsRelationshipDocument = resp.json().await?;
            let artworks: Vec<_> = artwork_resources(&doc.included).collect();
            for inc in &doc.included {
                let AlbumSearchIncluded::Album {
                    id,
                    attributes,
                    relationships,
                } = inc
                else {
                    continue;
                };
                let cover_url = select_artwork_url(relationships.cover_art.as_ref(), &artworks);
                albums.push(TidalAlbum::from(id.clone(), attributes.clone(), cover_url));
            }

            next = doc.links.and_then(|l| l.next);
            if next.is_none() {
                return Ok(albums);
            }
            tracing::debug!("tidal: fetching artist {id} albums page {}", page + 1);
        }

        tracing::warn!(
            "tidal: hit max pages ({MAX_PAGES}) exhausting artist {id} albums; \
             returning {} albums",
            albums.len()
        );
        Ok(albums)
    }

    pub async fn get_album_artists(&self, id: &str) -> Result<Vec<TidalArtist>, TidalError> {
        let url = format!("{TIDAL_BASE_URL}/albums/{id}?include=artists,artists.profileArt");
        let resp = self.send_with_retry(self.client.get(url)).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let doc: AlbumWithArtistsDocument = resp.json().await?;
        let album = doc.data.ok_or(TidalError::UnexpectedResponse)?;

        let relationships = album
            .relationships
            .and_then(|r| r.artists)
            .ok_or(TidalError::UnexpectedResponse)?;
        let artworks: Vec<_> = artwork_resources(&doc.included).collect();

        relationships
            .data
            .iter()
            .map(|rel| {
                doc.included
                    .iter()
                    .find_map(|inc| match inc {
                        AlbumSearchIncluded::Artist {
                            id,
                            attributes,
                            relationships,
                        } if id == &rel.id => Some(TidalArtist {
                            id: id.clone(),
                            name: attributes.name.clone(),
                            profile_image_url: select_profile_image_url(
                                relationships.as_ref(),
                                &artworks,
                            ),
                        }),
                        _ => None,
                    })
                    .ok_or(TidalError::UnexpectedResponse)
            })
            .collect()
    }

    pub async fn get_artist(&self, id: &str) -> Result<TidalArtist, TidalError> {
        let url = format!("{TIDAL_BASE_URL}/artists/{id}?include=profileArt");
        let resp = self.send_with_retry(self.client.get(url)).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let doc: tidal_response::ArtistSingleResource = resp.json().await?;
        let resource = doc.data.ok_or(TidalError::UnexpectedResponse)?;
        let artworks: Vec<_> = artwork_resources(&doc.included).collect();
        let profile_image_url =
            select_profile_image_url(resource.relationships.as_ref(), &artworks);

        Ok(TidalArtist {
            id: resource.id,
            name: resource.attributes.name,
            profile_image_url,
        })
    }

    pub async fn search_artists(&self, query: &str) -> Result<Vec<TidalArtist>, TidalError> {
        let url = format!("{TIDAL_BASE_URL}/searchResults");
        let request = self.client.get(url).query(&[
            ("filter[query]", query),
            ("include", "artists,artists.profileArt"),
        ]);

        let resp = self.send_with_retry(request).await?;
        if !resp.status().is_success() {
            tracing::error!("tidal: {} {}", resp.status(), resp.text().await?);
            return Err(TidalError::UnexpectedResponse);
        }

        let doc: ArtistSearchDocument = resp.json().await?;
        let search = doc.data.first().ok_or(TidalError::UnexpectedResponse)?;
        let artworks: Vec<_> = artwork_resources(&doc.included).collect();

        let relationships = search
            .relationships
            .as_ref()
            .and_then(|r| r.artists.as_ref())
            .ok_or(TidalError::UnexpectedResponse)?;

        relationships
            .data
            .iter()
            .map(|rel| {
                let (id, attributes, relationships) = doc
                    .included
                    .iter()
                    .find_map(|included| match included {
                        AlbumSearchIncluded::Artist {
                            id,
                            attributes,
                            relationships,
                        } if id == &rel.id => Some((id, attributes, relationships)),
                        _ => None,
                    })
                    .ok_or(TidalError::UnexpectedResponse)?;
                Ok(TidalArtist {
                    id: id.clone(),
                    name: attributes.name.clone(),
                    profile_image_url: select_profile_image_url(relationships.as_ref(), &artworks),
                })
            })
            .collect()
    }
}

fn artwork_resources<'a>(
    included: &'a [tidal_response::AlbumSearchIncluded],
) -> impl Iterator<Item = &'a tidal_response::ArtworkResource> {
    included.iter().filter_map(|included| match included {
        tidal_response::AlbumSearchIncluded::Artwork { resource } => Some(resource),
        _ => None,
    })
}

fn select_profile_image_url(
    relationships: Option<&tidal_response::ArtistIncludedRelationships>,
    artworks: &[&tidal_response::ArtworkResource],
) -> Option<String> {
    select_artwork_url(
        relationships.and_then(|relationships| relationships.profile_art.as_ref()),
        artworks,
    )
}

fn select_artwork_url(
    relationship: Option<&tidal_response::ArtworkRelationship>,
    artworks: &[&tidal_response::ArtworkResource],
) -> Option<String> {
    let relationship = relationship?;

    for related in relationship.data.as_ref()? {
        let Some(artwork) = artworks.iter().find(|artwork| artwork.id == related.id) else {
            continue;
        };
        if !artwork
            .attributes
            .media_type
            .as_deref()
            .is_some_and(|media_type| media_type == "IMAGE")
        {
            continue;
        }

        let mut largest_square: Option<(u32, &str)> = None;
        let mut smallest_large_square: Option<(u32, &str)> = None;
        for file in &artwork.attributes.files {
            let Some(href) = file.href.as_deref() else {
                continue;
            };
            let Some((width, height)) = artwork_file_dimensions(file) else {
                continue;
            };
            if width != height || !is_valid_https_url(href) {
                continue;
            }

            if largest_square.is_none_or(|(current_width, _)| width > current_width) {
                largest_square = Some((width, href));
            }
            if width >= 640
                && smallest_large_square.is_none_or(|(current_width, _)| width < current_width)
            {
                smallest_large_square = Some((width, href));
            }
        }

        if let Some((_, href)) = smallest_large_square.or(largest_square) {
            return Some(href.to_owned());
        }
    }

    None
}

fn artwork_file_dimensions(file: &tidal_response::ArtworkFile) -> Option<(u32, u32)> {
    match ((file.width, file.height), file.meta.as_ref()) {
        ((Some(width), Some(height)), _) => Some((width, height)),
        (_, Some(meta)) => Some((meta.width?, meta.height?)),
        _ => None,
    }
}

fn is_valid_https_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .is_some_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

#[derive(Debug, Clone)]
pub struct TidalArtist {
    pub id: String,
    pub name: String,
    pub profile_image_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TidalAlbum {
    pub id: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub release_date: Option<String>,
    pub popularity: f64,
    pub r#type: String,
}

impl TidalAlbum {
    fn from(id: String, attr: AlbumSearchIncludedAttributes, cover_url: Option<String>) -> Self {
        TidalAlbum {
            id,
            title: attr.title,
            cover_url,
            release_date: attr.release_date,
            popularity: attr.popularity,
            r#type: attr.r#type,
        }
    }
}

#[derive(Debug)]
pub struct ResolvedTidalSearchedAlbum {
    pub id: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub release_date: Option<String>,
    pub popularity: f64,
    pub artists: Vec<TidalArtist>,
    pub r#type: String,
}

fn resolve_searched_album(
    data: &AlbumSearchRelationshipsAlbumsData,
    attr: &AlbumSearchIncludedAttributes,
    artists: Vec<TidalArtist>,
    cover_url: Option<String>,
) -> ResolvedTidalSearchedAlbum {
    ResolvedTidalSearchedAlbum {
        id: data.id.clone(),
        title: attr.title.clone(),
        cover_url,
        release_date: attr.release_date.clone(),
        popularity: attr.popularity,
        artists,
        r#type: attr.r#type.clone(),
    }
}

mod tidal_response {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct OauthToken {
        pub access_token: String,
        pub expires_in: u64,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSearch {
        pub data: Vec<AlbumSearchData>,
        pub included: Vec<AlbumSearchIncluded>,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSearchData {
        pub relationships: AlbumSearchRelationships,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSearchRelationships {
        pub albums: AlbumSearchRelationshipsAlbums,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct AlbumSearchRelationshipsAlbums {
        pub data: Vec<AlbumSearchRelationshipsAlbumsData>,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSearchRelationshipsAlbumsData {
        pub id: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(tag = "type")]
    pub enum AlbumSearchIncluded {
        #[serde(rename = "albums")]
        Album {
            id: String,
            attributes: AlbumSearchIncludedAttributes,
            #[serde(default)]
            relationships: AlbumSearchIncludedRelationships,
        },
        #[serde(rename = "artists")]
        Artist {
            id: String,
            attributes: ArtistIncludedAttributes,
            #[serde(default)]
            relationships: Option<ArtistIncludedRelationships>,
        },
        #[serde(rename = "artworks")]
        Artwork {
            #[serde(flatten)]
            resource: ArtworkResource,
        },
        #[serde(other)]
        Unknown,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct AlbumSearchIncludedRelationships {
        #[serde(default)]
        pub artists: AlbumSearchRelationshipsAlbums,
        #[serde(default, rename = "coverArt")]
        pub cover_art: Option<ArtworkRelationship>,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct ArtworkResource {
        pub id: String,
        #[serde(default)]
        pub attributes: ArtworkAttributes,
    }

    #[derive(Debug, Deserialize, Clone, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct ArtworkAttributes {
        pub media_type: Option<String>,
        #[serde(default)]
        pub files: Vec<ArtworkFile>,
    }

    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct ArtworkFile {
        pub href: Option<String>,
        pub width: Option<u32>,
        pub height: Option<u32>,
        pub meta: Option<ArtworkFileMeta>,
    }

    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct ArtworkFileMeta {
        pub width: Option<u32>,
        pub height: Option<u32>,
    }

    #[derive(Debug, Deserialize, Clone, Default)]
    pub struct ArtworkRelationship {
        #[serde(default)]
        pub data: Option<Vec<ArtworkRelationshipData>>,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct ArtworkRelationshipData {
        pub id: String,
    }

    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct AlbumSearchIncludedAttributes {
        pub title: String,
        pub release_date: Option<String>,
        pub popularity: f64,
        pub r#type: String,
    }

    #[derive(Debug, Deserialize)]
    pub struct ArtistResource {
        pub id: String,
        pub attributes: ArtistIncludedAttributes,
        #[serde(default)]
        pub relationships: Option<ArtistIncludedRelationships>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct ArtistIncludedRelationships {
        #[serde(default, rename = "profileArt")]
        pub profile_art: Option<ArtworkRelationship>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ArtistSingleResource {
        pub data: Option<ArtistResource>,
        #[serde(default)]
        pub included: Vec<AlbumSearchIncluded>,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumSingleResource {
        pub data: Option<AlbumResource>,
        #[serde(default)]
        pub included: Vec<AlbumSearchIncluded>,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumResource {
        pub id: String,
        pub attributes: AlbumSearchIncludedAttributes,
        #[serde(default)]
        pub relationships: Option<AlbumSearchIncludedRelationships>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ArtistAlbumsRelationshipDocument {
        #[serde(default)]
        pub included: Vec<AlbumSearchIncluded>,
        #[serde(default)]
        pub links: Option<Links>,
    }

    #[derive(Debug, Deserialize, Default)]
    #[serde(default)]
    pub struct Links {
        pub next: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumWithArtistsDocument {
        pub data: Option<AlbumWithArtistsResource>,
        #[serde(default)]
        pub included: Vec<AlbumSearchIncluded>,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumWithArtistsResource {
        pub relationships: Option<AlbumWithArtistsRelationships>,
    }

    #[derive(Debug, Deserialize)]
    pub struct AlbumWithArtistsRelationships {
        pub artists: Option<AlbumSearchRelationshipsAlbums>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ArtistSearchDocument {
        pub data: Vec<ArtistSearchData>,
        #[serde(default)]
        pub included: Vec<AlbumSearchIncluded>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ArtistSearchData {
        pub relationships: Option<ArtistSearchRelationships>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ArtistSearchRelationships {
        pub artists: Option<AlbumSearchRelationshipsAlbums>,
    }

    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct ArtistIncludedAttributes {
        pub name: String,
    }
}

#[cfg(test)]
mod tests {
    use super::tidal_response::{
        AlbumSearchIncluded, AlbumSearchIncludedAttributes, AlbumWithArtistsDocument,
        ArtistAlbumsRelationshipDocument, ArtistSingleResource, ArtworkRelationship,
        ArtworkRelationshipData, ArtworkResource,
    };
    use super::{artwork_resources, select_artwork_url, select_profile_image_url};

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
}
