use std::sync::Arc;

use reqwest::{ClientBuilder, StatusCode};
use tracing::{debug, instrument};

use crate::downloaders::RateLimit;

pub mod artist;
pub mod release_group;

static BASE_URL: &'static str = "https://musicbrainz.org/ws/2";

#[derive(thiserror::Error, Debug)]
pub enum RequestError {
    #[error("Error sending or receiving request: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Musicbrainz API returned an error: {0} {1}")]
    MusicbrainzError(StatusCode, String),
}

#[derive(Debug, Clone)]
pub struct Musicbrainz {
    client: Arc<reqwest::Client>,
    rate_limit: RateLimit,
}

impl Musicbrainz {
    pub fn new() -> Musicbrainz {
        debug!("Creating musicbrainz client");
        Musicbrainz {
            client: Arc::new(
                ClientBuilder::default()
                    .user_agent("platen/1.0.0 (https://github.com/jorikvanveen)")
                    .build()
                    .unwrap(),
            ),
            rate_limit: RateLimit::new(1200),
        }
    }

    #[instrument]
    async fn fetch_with_retry(&self, url: &str) -> Result<reqwest::Response, RequestError> {
        loop {
            self.rate_limit.wait().await;
            let resp = self.client.get(url).send().await?;
            if resp.status() != StatusCode::SERVICE_UNAVAILABLE {
                break Ok(resp);
            }
        }
    }
}
