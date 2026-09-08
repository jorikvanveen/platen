use async_trait::async_trait;
use axum::{Json, extract::State};
use reqwest::StatusCode;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, sea_query::Expr};
use tracing::error;
use url::Url;

use crate::{
    app::AppState,
    entity::{album, artist},
    services::tidal::{Tidal, TidalError},
};

pub mod dto {
    use serde::Serialize;
    use ts_rs::TS;

    #[derive(Debug, Default, PartialEq, Eq, Serialize, TS)]
    #[ts(export)]
    pub struct ArtworkRefreshCounts {
        pub updated: u32,
        pub already_present: u32,
        pub unavailable: u32,
        pub failed: u32,
    }

    #[derive(Debug, Default, PartialEq, Eq, Serialize, TS)]
    #[ts(export)]
    pub struct ArtworkRefreshSummary {
        pub albums: ArtworkRefreshCounts,
        pub artists: ArtworkRefreshCounts,
    }
}

#[async_trait]
trait ArtworkSource {
    async fn album_cover(&self, id: &str) -> Result<Option<String>, TidalError>;
    async fn artist_profile_image(&self, id: &str) -> Result<Option<String>, TidalError>;

    async fn refresh_artwork_with(
        &self,
        db: &DatabaseConnection,
    ) -> Result<dto::ArtworkRefreshSummary, DbErr> {
        let albums = album::Entity::find().all(db).await?;
        let artists = artist::Entity::find().all(db).await?;
        let mut summary = dto::ArtworkRefreshSummary::default();

        for model in albums {
            if model.cover_url.is_some() {
                summary.albums.already_present += 1;
                continue;
            }

            let url = match self.album_cover(&model.id).await {
                Ok(Some(url)) if is_valid_https_url(&url) => url,
                Ok(_) => {
                    summary.albums.unavailable += 1;
                    continue;
                }
                Err(e) => {
                    error!("Could not fetch artwork for album {}: {e:#?}", model.id);
                    summary.albums.failed += 1;
                    continue;
                }
            };

            match album::Entity::update_many()
                .col_expr(album::Column::CoverUrl, Expr::value(url))
                .filter(album::Column::Id.eq(&model.id))
                .filter(album::Column::CoverUrl.is_null())
                .exec(db)
                .await
            {
                Ok(result) if result.rows_affected == 1 => summary.albums.updated += 1,
                Ok(_) => summary.albums.already_present += 1,
                Err(e) => {
                    error!("Could not save artwork for album {}: {e:#?}", model.id);
                    summary.albums.failed += 1;
                }
            }
        }

        for model in artists {
            if model.profile_image_url.is_some() {
                summary.artists.already_present += 1;
                continue;
            }

            let url = match self.artist_profile_image(&model.id).await {
                Ok(Some(url)) if is_valid_https_url(&url) => url,
                Ok(_) => {
                    summary.artists.unavailable += 1;
                    continue;
                }
                Err(e) => {
                    error!("Could not fetch artwork for artist {}: {e:#?}", model.id);
                    summary.artists.failed += 1;
                    continue;
                }
            };

            match artist::Entity::update_many()
                .col_expr(artist::Column::ProfileImageUrl, Expr::value(url))
                .filter(artist::Column::Id.eq(&model.id))
                .filter(artist::Column::ProfileImageUrl.is_null())
                .exec(db)
                .await
            {
                Ok(result) if result.rows_affected == 1 => summary.artists.updated += 1,
                Ok(_) => summary.artists.already_present += 1,
                Err(e) => {
                    error!("Could not save artwork for artist {}: {e:#?}", model.id);
                    summary.artists.failed += 1;
                }
            }
        }

        Ok(summary)
    }
}

#[async_trait]
impl ArtworkSource for Tidal {
    async fn album_cover(&self, id: &str) -> Result<Option<String>, TidalError> {
        self.get_album_cover(id).await
    }

    async fn artist_profile_image(&self, id: &str) -> Result<Option<String>, TidalError> {
        Ok(self.get_artist(id).await?.profile_image_url)
    }
}

pub async fn refresh_artwork(
    State(AppState { tidal, db, .. }): State<AppState>,
) -> Result<Json<dto::ArtworkRefreshSummary>, StatusCode> {
    tidal
        .refresh_artwork_with(&db)
        .await
        .map(Json)
        .map_err(|e| {
            error!("Could not load Catalog records for artwork refresh: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

fn is_valid_https_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .is_some_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

#[cfg(test)]
#[path = "../tests/routes/catalog.rs"]
mod tests;
