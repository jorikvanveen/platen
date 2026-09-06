use axum::{
    Router,
    routing::{delete, get, post},
};
use sea_orm::DatabaseConnection;

use crate::{
    routes,
    services::{catalog::ScanCoordinator, download_queue::DownloadQueue, tidal::Tidal},
};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) tidal: Tidal,
    pub(crate) queue: DownloadQueue,
    pub(crate) scan: ScanCoordinator,
    pub(crate) db: DatabaseConnection,
}

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/artists", get(routes::artist::list))
        .route("/artists/{id}", get(routes::artist::get))
        .route(
            "/artists/{artist_id}/albums/{album_id}",
            post(routes::album::create_artist_scoped),
        )
        .route(
            "/albums/{album_id}",
            post(routes::album::create).delete(routes::album::delete),
        )
        .route(
            "/albums/{album_id}/deletion-preview",
            get(routes::album::deletion_preview),
        )
        .route(
            "/catalog/refresh-artwork",
            post(routes::catalog::refresh_artwork),
        )
        .route(
            "/catalog/scan",
            get(routes::catalog::scan_status).post(routes::catalog::start_scan),
        )
        .route(
            "/artists/{artist_id}/albums",
            get(routes::album::fetch_all_artist_albums),
        )
        .route(
            "/albums/refresh-release-dates",
            get(routes::album::refresh_release_dates),
        )
        .route("/albums/{album_id}/download", post(routes::album::download))
        .route("/downloads", get(routes::download::list))
        .route("/downloads/{job_id}", delete(routes::download::cancel))
        .route("/tidal/search/artists", get(routes::tidal::search_artists))
        .route("/tidal/search/albums", get(routes::tidal::search_albums))
        .route("/tidal/artists/{id}", get(routes::tidal::get_artist_albums))
        .route("/", get(|| async { "Hello world" }))
        .with_state(state)
}

#[cfg(test)]
#[path = "tests/app.rs"]
mod tests;
