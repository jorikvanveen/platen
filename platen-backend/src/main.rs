use std::{path::PathBuf, sync::Arc, time::Duration};

use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};
use tokio::{net::TcpListener, time::Instant};
use tracing_subscriber::filter::EnvFilter;

use crate::{
    app::{AppState, router},
    config::Config,
    services::{
        download_queue::DownloadQueue, downloaders::antra::Antra, import::ScanCoordinator,
        music_directory::MusicDirectory, rate_limit::RateLimit, tidal::Tidal,
    },
};

mod app;
mod config;
#[allow(unused)]
mod entity;
mod routes;
mod services;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod test_support;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let server_started_at = Instant::now();
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("sqlx::query=off".parse()?);
    tracing_subscriber::fmt().with_env_filter(filter).init();
    tracing::info!("Starting platen");

    let config = Config::load()?;
    tracing::info!("Loaded configuration");

    let tidal_rate_limit = RateLimit::new(Duration::from_secs(1));
    let tidal = Tidal::new(
        config.tidal_client_id.clone(),
        config.tidal_client_secret.clone(),
        config.tidal_country_code.clone(),
        tidal_rate_limit,
    );
    tidal.login().await?;

    let antra = Arc::new(Antra::new(&config, tidal.clone()));
    antra.login().await?;

    let db: DatabaseConnection = Database::connect(&config.database_url).await?;
    Migrator::up(&db, None).await?;

    let listener = TcpListener::bind(&config.bind_address).await?;
    let music_directory = MusicDirectory::new(PathBuf::from(&config.music_dir));
    let (queue, worker_handle) =
        DownloadQueue::start(db.clone(), music_directory.clone(), antra.clone());
    let reauthentication_queue = queue.clone();
    let reauthentication_handle = tokio::spawn(async move {
        antra
            .reauthenticate_periodically(&reauthentication_queue, server_started_at)
            .await;
    });
    let scan = ScanCoordinator::new(music_directory, db.clone(), Arc::new(tidal.clone()));
    let app = router(AppState {
        tidal,
        queue,
        scan,
        db,
    });

    let server_result = tokio::select! {
        result = axum::serve(listener, app) => result,
        result = tokio::signal::ctrl_c() => result,
    };
    worker_handle.abort();
    reauthentication_handle.abort();
    let _ = worker_handle.await;
    let _ = reauthentication_handle.await;
    server_result?;

    Ok(())
}
