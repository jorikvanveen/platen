use std::{error::Error, path::Path, sync::Arc, time::Duration};

use tokio::{sync::{Barrier, Semaphore, mpsc}, task, time::sleep};
use tracing::info;

use crate::musicbrainz::release::Release;

pub trait Downloader {
    type Error: Error;
    async fn download_release(
        &self,
        release: &Release,
        destination: &Path,
    ) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug)]
pub struct RateLimit {
    semaphore: Arc<Semaphore>,
    cooldown_ms: u64
}

impl RateLimit {
    pub fn new(cooldown_ms: u64) -> RateLimit {
        let semaphore = Arc::new(Semaphore::new(1));
        RateLimit { semaphore, cooldown_ms }
    }

    pub async fn wait(&self) {
        let permit = Arc::clone(&self.semaphore).acquire_owned().await.unwrap();
        let duration = Duration::from_millis(self.cooldown_ms);
        task::spawn(async move {
            // Make sure that next acquisition goes through at least `cooldown_ms` later
            sleep(duration).await;
            drop(permit);
        });
    }
}

pub mod youtube;
