use std::collections::HashMap;

use reqwest::{ClientBuilder, StatusCode};
use tracing::{error, instrument};

use jellyfin_response::{BaseItemDto, QueryResult};

const PAGE_SIZE: usize = 1000;

#[derive(thiserror::Error, Debug)]
pub enum JellyfinError {
    #[error("Jellyfin returned an error: {0} {1}")]
    Http(StatusCode, String),

    #[error("Failed to authenticate with Jellyfin")]
    Auth,

    #[error("Jellyfin is unreachable")]
    Unreachable,

    #[error("Failed to parse Jellyfin response: {0}")]
    Parse(#[from] serde_json::Error),
}

impl From<reqwest::Error> for JellyfinError {
    fn from(_: reqwest::Error) -> Self {
        JellyfinError::Unreachable
    }
}

#[derive(Debug, Clone)]
pub struct Jellyfin {
    client: reqwest::Client,
    url: String,
    api_key: String,
    user_id: String,
}

impl Jellyfin {
    pub fn new(url: String, api_key: String, user_id: String) -> Self {
        Self {
            client: ClientBuilder::new().build().unwrap(),
            url: url.trim_end_matches('/').to_string(),
            api_key,
            user_id,
        }
    }

    fn request(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.url, path);
        self.client
            .get(url)
            .header("Authorization", format!("MediaBrowser Token=\"{}\"", self.api_key))
    }

    #[instrument]
    pub async fn list_albums(&self) -> Result<Vec<JellyfinAlbum>, JellyfinError> {
        let mut albums = Vec::new();
        let mut start_index = 0usize;

        loop {
            let path = format!(
                "/Items?userId={}&includeItemTypes=MusicAlbum&recursive=true&fields=ProviderIds,Genres&enableImages=false&enableUserData=false&enableTotalRecordCount=true&startIndex={}&limit={}",
                self.user_id, start_index, PAGE_SIZE,
            );

            let resp = self.request(&path).send().await?;
            let status = resp.status();
            if status == StatusCode::UNAUTHORIZED {
                return Err(JellyfinError::Auth);
            }
            if status == StatusCode::SERVICE_UNAVAILABLE {
                return Err(JellyfinError::Unreachable);
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                error!("Jellyfin items request failed: {status} {body}");
                return Err(JellyfinError::Http(status, body));
            }

            let bytes = resp.bytes().await?;
            let result: QueryResult<BaseItemDto> = serde_json::from_slice(&bytes)?;
            let page_len = result.items.len();
            let total = result.total_record_count.unwrap_or(page_len);

            albums.extend(result.items.into_iter().map(JellyfinAlbum::from));

            if start_index + page_len >= total {
                break;
            }
            start_index += page_len;
        }

        Ok(albums)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct JellyfinAlbum {
    pub id: String,
    pub name: String,
    pub provider_ids: HashMap<String, String>,
    pub production_year: Option<i32>,
}

impl From<BaseItemDto> for JellyfinAlbum {
    fn from(dto: BaseItemDto) -> Self {
        JellyfinAlbum {
            id: dto.id.unwrap_or_default(),
            name: dto.name.unwrap_or_default(),
            provider_ids: dto.provider_ids.unwrap_or_default(),
            production_year: dto.production_year,
        }
    }
}

mod jellyfin_response {
    use std::collections::HashMap;

    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    pub struct QueryResult<T> {
        #[serde(default = "Vec::new")]
        pub items: Vec<T>,
        #[serde(default)]
        pub total_record_count: Option<usize>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    #[serde(rename_all = "PascalCase")]
    pub struct BaseItemDto {
        #[serde(default)]
        pub id: Option<String>,
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub provider_ids: Option<HashMap<String, String>>,
        #[serde(default)]
        pub production_year: Option<i32>,
    }
}
