use std::{path::{Path, PathBuf}, sync::Arc, time::Duration};

use axum::{Router, extract::FromRef, routing::{get, post}};
use migration::{Migrator, MigratorTrait};
use reqwest::ClientBuilder;
use rustypipe::{client::RustyPipe, model::{MusicItem, MusicSearchResult, SearchResult, TrackItem}};
use sea_orm::{Database, DatabaseConnection};
use tokio::{net::TcpListener, task, time::sleep};
use tracing::{Level, instrument};

use crate::{downloaders::{Downloader, RateLimit, youtube::Youtube}, musicbrainz::Musicbrainz};

mod downloaders;
mod musicbrainz;
mod routes;
mod entity;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting platen");

    let db: DatabaseConnection = Database::connect("sqlite://./db.sqlite?mode=rwc").await?;
    Migrator::up(&db, None).await?;

    let state = AppState { musicbrainz: Musicbrainz::new(), db };
        
    let app = Router::new()
        .route("/artist/{id}", get(routes::artist::get))
        .route("/artist/{id}", post(routes::artist::create))
        .route("/", get(|| async { "Hello world" }))
        .with_state(state);
    let listener = TcpListener::bind("0.0.0.0:3000").await?;

    axum::serve(listener, app).await?;
    
    Ok(())
}

#[derive(Clone)]
struct AppState {
    musicbrainz: Musicbrainz,
    db: DatabaseConnection
}
