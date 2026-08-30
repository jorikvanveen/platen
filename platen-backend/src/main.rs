use std::{path::PathBuf, sync::Arc};

use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};
use tokio::net::TcpListener;
use tracing_subscriber::filter::EnvFilter;

use crate::{
    app::{AppState, router},
    config::Config,
    services::{download_queue::DownloadQueue, downloaders::antra::Antra, tidal::Tidal},
};

mod app;
mod config;
#[allow(unused)]
mod entity;
mod routes;
mod services;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("sqlx::query=off".parse()?);
    tracing_subscriber::fmt().with_env_filter(filter).init();
    tracing::info!("Starting platen");

    let config = Config::load()?;
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
    let (queue, worker_handle) = DownloadQueue::start(
        db.clone(),
        PathBuf::from(&config.music_dir),
        Arc::new(antra),
    );
    let app = router(AppState { tidal, queue, db });
    let listener = TcpListener::bind(&bind_address).await?;

    let server_result = tokio::select! {
        result = axum::serve(listener, app) => result,
        result = tokio::signal::ctrl_c() => {
            result.map_err(color_eyre::Report::from)?;
            Ok(())
        }
    };
    worker_handle.abort();
    let _ = worker_handle.await;
    server_result?;

    Ok(())
}
