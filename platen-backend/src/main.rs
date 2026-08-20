use std::path::PathBuf;

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
use tokio::{net::TcpListener, process::Command};

use crate::{
    downloaders::{Downloader, antra::Antra},
    musicbrainz::Musicbrainz,
};

mod downloaders;
mod entity;
mod musicbrainz;
mod routes;

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

    let antra = Antra::new(&config);
    antra.login().await?;

    let musicbrainz = Musicbrainz::new();

    let db: DatabaseConnection = Database::connect(&config.database_url).await?;
    Migrator::up(&db, None).await?;

    let bind_address = config.bind_address.clone();
    let state = AppState {
        musicbrainz,
        db,
        config,
        antra,
    };
    let app = Router::new()
        .route("/artist", get(routes::artist::list))
        .route("/artist/{id}", get(routes::artist::get))
        .route("/artist/{id}", post(routes::artist::create))
        .route(
            "/artist/{artist_id}/release-group/{release_group_id}",
            post(routes::release_group::create),
        )
        .route(
            "/artist/{artist_id}/release-groups",
            get(routes::release_group::fetch_all),
        )
        .route(
            "/artist/{artist_id}/release-group/{release_group_id}/download",
            post(routes::release_group::download),
        )
        .route("/mb/search_artist/{query}", get(routes::mb::search_artist))
        .route("/mb/artist/{artist_id}", get(routes::mb::get_artist))
        .route(
            "/mb/artist/{artist_id}/release-groups",
            get(routes::mb::get_artist_release_groups),
        )
        .route("/", get(|| async { "Hello world" }))
        .with_state(state);
    let listener = TcpListener::bind(&bind_address).await?;

    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Clone)]
struct AppState {
    musicbrainz: Musicbrainz,
    antra: Antra,
    db: DatabaseConnection,
    config: Config,
}
