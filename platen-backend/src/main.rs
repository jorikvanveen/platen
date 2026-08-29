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
use tokio::net::TcpListener;

use crate::services::{downloaders::antra::Antra, tidal::Tidal};

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

    let tidal = Tidal::new(
        config.tidal_client_id.clone(),
        config.tidal_client_secret.clone(),
    );
    tidal.login().await?;

    let antra = Antra::new(&config);
    antra.login().await?;

    let db: DatabaseConnection = Database::connect(&config.database_url).await?;
    Migrator::up(&db, None).await?;

    let bind_address = config.bind_address.clone();
    let state = AppState {
        tidal,
        antra,
        db,
        config,
    };
    let app = app_router(state);
    let listener = TcpListener::bind(&bind_address).await?;

    axum::serve(listener, app).await?;

    Ok(())
}

fn app_router(state: AppState) -> Router {
    Router::new()
        .route("/artists", get(routes::artist::list))
        .route("/artists/{id}", get(routes::artist::get))
        .route(
            "/artists/{artist_id}/albums/{album_id}",
            post(routes::album::create_artist_scoped),
        )
        .route("/albums/{album_id}", post(routes::album::create))
        .route(
            "/artists/{artist_id}/albums",
            get(routes::album::fetch_all_artist_albums),
        )
        .route(
            "/albums/refresh-release-dates",
            get(routes::album::refresh_release_dates),
        )
        .route("/albums/{album_id}/download", post(routes::album::download))
        .route("/tidal/search/artists", get(routes::tidal::search_artists))
        .route("/tidal/search/albums", get(routes::tidal::search_albums))
        .route("/tidal/artists/{id}", get(routes::tidal::get_artist_albums))
        .route("/", get(|| async { "Hello world" }))
        .with_state(state)
}

#[derive(Clone)]
struct AppState {
    tidal: Tidal,
    antra: Antra,
    db: DatabaseConnection,
    config: Config,
}
