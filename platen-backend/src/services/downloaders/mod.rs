use std::{error::Error, path::Path, sync::Arc, time::Duration};

use tokio::{sync::Semaphore, task, time::sleep};

pub mod antra;

pub trait Downloader {
    type Error: Error;
    async fn download_album(
        &self,
        album: &crate::entity::album::Model,
        destination: &Path,
    ) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug)]
pub struct RateLimit {
    semaphore: Arc<Semaphore>,
    cooldown_ms: u64,
}

impl RateLimit {
    pub fn new(cooldown_ms: u64) -> RateLimit {
        let semaphore = Arc::new(Semaphore::new(1));
        RateLimit {
            semaphore,
            cooldown_ms,
        }
    }

    pub async fn wait(&self) {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .expect("RateLimit owns the semaphore; it cannot be closed while RateLimit is alive");
        let duration = Duration::from_millis(self.cooldown_ms);
        task::spawn(async move {
            sleep(duration).await;
            drop(permit);
        });
    }
}
