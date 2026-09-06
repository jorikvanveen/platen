use crate::services::tidal::{self, TidalCatalog};

pub(crate) struct EmptyTidalCatalog;

#[async_trait::async_trait]
impl TidalCatalog for EmptyTidalCatalog {
    async fn find_album(
        &self,
        _: &str,
    ) -> Result<Vec<tidal::ResolvedTidalSearchedAlbum>, tidal::TidalError> {
        Ok(Vec::new())
    }

    async fn get_album(&self, _: &str) -> Result<tidal::TidalAlbum, tidal::TidalError> {
        Err(tidal::TidalError::UnexpectedResponse)
    }

    async fn get_album_cover(&self, _: &str) -> Result<Option<String>, tidal::TidalError> {
        Err(tidal::TidalError::UnexpectedResponse)
    }

    async fn get_album_artists(
        &self,
        _: &str,
    ) -> Result<Vec<tidal::TidalArtist>, tidal::TidalError> {
        Err(tidal::TidalError::UnexpectedResponse)
    }
}
