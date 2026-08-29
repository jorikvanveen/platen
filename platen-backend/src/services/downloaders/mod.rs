use std::{error::Error, path::Path};

use async_trait::async_trait;

pub mod antra;

#[async_trait]
pub trait Downloader: Send + Sync {
    async fn download_album(
        &self,
        album: &crate::entity::album::Model,
        destination: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;
}
