use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};
use figment::{
    Figment,
    providers::{Env, Format, Toml},
};
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};
use serde::Deserialize;
use tokio::{net::TcpListener, sync::Mutex};

use crate::services::{
    downloaders::antra::Antra, jellyfin::Jellyfin, musicbrainz::Musicbrainz, tidal::Tidal,
};

#[allow(unused)]
mod entity;
mod routes;
mod services;

#[derive(Debug, Deserialize, Clone)]
struct Config {
    database_url: String,
    bind_address: String,
    antra_password: String,
    antra_username: String,
    tidal_client_id: String,
    tidal_client_secret: String,
    music_dir: String,
    jellyfin_url: String,
    jellyfin_api_key: String,
    jellyfin_user_id: String,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting platen");

    let config: Config = Figment::new()
        .merge(Toml::file("platen.toml"))
        .merge(Env::prefixed("PLATEN_").split("__"))
        .extract()?;
    tracing::info!("Loaded configuration");

    let tidal = Arc::new(Mutex::new(Tidal::new(
        config.tidal_client_id.clone(),
        config.tidal_client_secret.clone(),
    )));
    tidal.lock().await.login().await?;

    let antra = Antra::new(&config);
    antra.login().await?;

    let musicbrainz = Musicbrainz::new();

    let jellyfin = Jellyfin::new(
        config.jellyfin_url.clone(),
        config.jellyfin_api_key.clone(),
        config.jellyfin_user_id.clone(),
    );

    let db: DatabaseConnection = Database::connect(&config.database_url).await?;
    Migrator::up(&db, None).await?;

    let bind_address = config.bind_address.clone();
    let state = AppState {
        musicbrainz,
        jellyfin,
        tidal,
        antra,
        db,
        config,
    };
    let app = Router::new()
        .route("/artists", get(routes::artist::list))
        .route("/artists/{id}", get(routes::artist::get))
        .route("/artists/{id}", post(routes::artist::create))
        .route(
            "/artists/{artist_id}/albums/{album_id}",
            post(routes::album::create),
        )
        .route("/artists/{artist_id}/albums", get(routes::album::fetch_all))
        .route(
            "/albums/refresh-release-dates",
            get(routes::album::refresh_release_dates),
        )
        .route(
            "/artists/{artist_id}/albums/{album_id}/download",
            post(routes::album::download),
        )
        .route("/tidal/search/artists", get(routes::tidal::search_artists))
        .route("/tidal/artists/{id}", get(routes::tidal::get_artist_albums))
        .route("/jellyfin/import", post(routes::jellyfin::import))
        .route("/", get(|| async { "Hello world" }))
        .with_state(state);
    let listener = TcpListener::bind(&bind_address).await?;

    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Clone)]
struct AppState {
    musicbrainz: Musicbrainz,
    jellyfin: Jellyfin,
    tidal: Arc<Mutex<Tidal>>,
    antra: Antra,
    db: DatabaseConnection,
    config: Config,
}
