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
