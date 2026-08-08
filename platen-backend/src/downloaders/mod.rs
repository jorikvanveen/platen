use std::{error::Error, path::Path, time::Duration};

use tokio::{sync::mpsc, task, time::sleep};

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
    sender: mpsc::Sender<()>,
}

impl RateLimit {
    pub fn new(cooldown_ms: u64) -> RateLimit {
        let (sender, mut receiver) = mpsc::channel::<()>(1);

        task::spawn(async move {
            loop {
                sleep(Duration::from_millis(cooldown_ms)).await;
                receiver.recv().await;
                tracing::debug!("msg received");
            }
        });

        RateLimit { sender }
    }

    pub async fn wait(&self) {
        tracing::debug!("msg sent");
        self.sender.send(()).await.unwrap();
        tracing::debug!("msg acknowledged");
    }
}

pub mod youtube;
