use async_trait::async_trait;
use axum::{Json, extract::State, response::IntoResponse};
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

    use crate::services::catalog::{ScanPhase, ScanSnapshot, ScanSummary};

    #[derive(Debug, PartialEq, Eq, Serialize, TS)]
    #[serde(rename_all = "snake_case")]
    #[ts(export, rename_all = "snake_case")]
    pub enum CatalogScanPhase {
        Scanning,
        Matching,
        Completed,
        Failed,
    }

    impl From<ScanPhase> for CatalogScanPhase {
        fn from(phase: ScanPhase) -> Self {
            match phase {
                ScanPhase::Scanning => Self::Scanning,
                ScanPhase::Matching => Self::Matching,
                ScanPhase::Completed => Self::Completed,
                ScanPhase::Failed => Self::Failed,
            }
        }
    }

    #[derive(Debug, PartialEq, Eq, Serialize, TS)]
    #[ts(export)]
    pub struct CatalogScanSummary {
        pub album_directories_found: usize,
        pub candidates_processed: usize,
        pub candidates_total: usize,
        pub albums_imported: usize,
        pub locations_attached: usize,
        pub locations_changed: usize,
        pub unchanged_locations: usize,
        pub locations_cleared: usize,
        pub unmatched_candidates: usize,
        pub ambiguous_matches: usize,
        pub duplicate_locations: usize,
        pub skipped_directories: usize,
        pub failures: usize,
        pub filesystem_errors: usize,
    }

    impl From<ScanSummary> for CatalogScanSummary {
        fn from(summary: ScanSummary) -> Self {
            Self {
                album_directories_found: summary.album_directories_found,
                candidates_processed: summary.candidates_processed,
                candidates_total: summary.candidates_total,
                albums_imported: summary.albums_imported,
                locations_attached: summary.locations_attached,
                locations_changed: summary.locations_changed,
                unchanged_locations: summary.unchanged_locations,
                locations_cleared: summary.locations_cleared,
                unmatched_candidates: summary.unmatched_candidates,
                ambiguous_matches: summary.ambiguous_matches,
                duplicate_locations: summary.duplicate_locations,
                skipped_directories: summary.skipped_directories,
                failures: summary.failures,
                filesystem_errors: summary.filesystem_errors,
            }
        }
    }

    #[derive(Debug, PartialEq, Eq, Serialize, TS)]
    #[ts(export)]
    pub struct CatalogScan {
        pub phase: CatalogScanPhase,
        pub summary: CatalogScanSummary,
        pub failure_reason: Option<String>,
    }

    impl From<ScanSnapshot> for CatalogScan {
        fn from(scan: ScanSnapshot) -> Self {
            Self {
                phase: scan.phase.into(),
                summary: scan.summary.into(),
                failure_reason: scan.failure_reason,
            }
        }
    }

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

pub async fn start_scan(State(AppState { scan, .. }): State<AppState>) -> impl IntoResponse {
    match scan.start().await {
        Ok(status) => (StatusCode::ACCEPTED, Json(dto::CatalogScan::from(status))),
        Err(active) => (StatusCode::CONFLICT, Json(dto::CatalogScan::from(active))),
    }
}

pub async fn scan_status(
    State(AppState { scan, .. }): State<AppState>,
) -> Json<Option<dto::CatalogScan>> {
    Json(scan.snapshot().await.map(Into::into))
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
