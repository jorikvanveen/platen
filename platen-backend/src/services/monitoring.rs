use std::{collections::HashSet, sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait, sea_query::Expr,
};
use tokio::{task::JoinHandle, time::MissedTickBehavior};

use super::{
    catalog::{PrepareAlbumError, persist_album_in_transaction, prepare_album},
    discovery::{AlbumIdentity, select_candidates},
    download_queue::{DownloadQueue, QueueError},
    tidal::{TidalAlbum, TidalCatalog},
};
use crate::entity::{album, album_artist, artist, artist_known_album};

const CHECK_INTERVAL: TimeDelta = TimeDelta::hours(6);

pub(crate) fn start(
    db: DatabaseConnection,
    tidal: Arc<dyn TidalCatalog>,
    queue: DownloadQueue,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Sweeping the Catalog also finds Artists introduced by scans and future import paths.
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = check_due_artists(&db, tidal.as_ref(), &queue, Utc::now).await {
                tracing::error!(%error, "Could not find Artists due for a discography check");
            }
        }
    })
}

pub(crate) async fn check_due_artists(
    db: &DatabaseConnection,
    tidal: &dyn TidalCatalog,
    queue: &DownloadQueue,
    now: impl Fn() -> DateTime<Utc>,
) -> Result<(), DbErr> {
    let latest_due_attempt_at = now()
        .checked_sub_signed(CHECK_INTERVAL)
        .ok_or_else(|| DbErr::Custom("Artist check time exceeds the supported range".to_owned()))?;
    let artist_ids = artist::Entity::find()
        .select_only()
        .column(artist::Column::Id)
        .filter(
            Condition::any()
                .add(artist::Column::LastCheckAttemptAt.is_null())
                .add(artist::Column::LastCheckAttemptAt.lte(latest_due_attempt_at)),
        )
        .into_tuple::<String>()
        .all(db)
        .await?;
    for artist_id in artist_ids {
        if let Err(error) = check_artist(db, tidal, queue, &artist_id, now()).await {
            tracing::error!(%artist_id, %error, "Could not persist Artist Monitoring Baseline");
        }
    }
    Ok(())
}

async fn check_artist(
    db: &DatabaseConnection,
    tidal: &dyn TidalCatalog,
    queue: &DownloadQueue,
    artist_id: &str,
    now: DateTime<Utc>,
) -> Result<(), DbErr> {
    // Record the attempt before fetching so failures and restarts retain the retry delay.
    artist::Entity::update_many()
        .col_expr(artist::Column::LastCheckAttemptAt, Expr::value(now))
        .filter(artist::Column::Id.eq(artist_id))
        .exec(db)
        .await?;
    let albums = match tidal.get_artist_albums(artist_id).await {
        Ok(albums) => albums,
        Err(error) => {
            tracing::warn!(%artist_id, %error, "Artist discography fetch failed; will retry");
            return Ok(());
        }
    };
    let transaction = db.begin().await?;
    let artist = artist::Entity::find_by_id(artist_id)
        .one(&transaction)
        .await?
        .ok_or_else(|| DbErr::RecordNotFound(format!("Artist {artist_id} was not found")))?;
    let known_identities: HashSet<_> = artist_known_album::Entity::find()
        .filter(artist_known_album::Column::ArtistId.eq(artist_id))
        .all(&transaction)
        .await?
        .into_iter()
        .map(|entry| AlbumIdentity::new(&entry.normalized_title, &entry.release_type))
        .collect();
    let observed_identities: HashSet<_> = albums.iter().map(AlbumIdentity::from).collect();
    for identity in observed_identities.difference(&known_identities) {
        artist_known_album::Entity::insert(artist_known_album::ActiveModel {
            artist_id: Set(artist_id.to_owned()),
            normalized_title: Set(identity.normalized_title.clone()),
            release_type: Set(identity.release_type.clone()),
        })
        .exec(&transaction)
        .await?;
    }
    let initializing = artist.monitoring_baseline_initialized_at.is_none();
    if initializing {
        artist::Entity::update_many()
            .col_expr(
                artist::Column::MonitoringBaselineInitializedAt,
                Expr::value(now),
            )
            .filter(artist::Column::Id.eq(artist_id))
            .exec(&transaction)
            .await?;
    }
    // Observations stay known even if automatic work fails or the process stops before handoff.
    transaction.commit().await?;
    tracing::info!(
        %artist_id,
        initializing,
        observed = observed_identities.len(),
        newly_observed = observed_identities.difference(&known_identities).count(),
        "Updated Artist Monitoring Baseline"
    );
    if initializing || !artist.monitored {
        return Ok(());
    }
    let new_albums = albums
        .into_iter()
        .filter(|album| !known_identities.contains(&AlbumIdentity::from(album)));
    let candidates = match select_candidates(new_albums) {
        Ok(candidates) => candidates,
        Err(error) => {
            tracing::error!(%artist_id, %error, "Could not select monitoring discoveries; will not retry these observations");
            return Ok(());
        }
    };
    for candidate in candidates {
        if let Err(error) = hand_off_discovery(db, tidal, queue, artist_id, &candidate).await {
            tracing::error!(%artist_id, album_id = %candidate.id, %error, "Could not hand off monitoring discovery; will not retry this observation");
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum HandoffError {
    #[error(transparent)]
    Database(#[from] DbErr),
    #[error(transparent)]
    PrepareAlbum(#[from] PrepareAlbumError),
    #[error(transparent)]
    Queue(#[from] QueueError),
}

async fn is_monitored(db: &impl ConnectionTrait, artist_id: &str) -> Result<bool, DbErr> {
    Ok(artist::Entity::find_by_id(artist_id)
        .one(db)
        .await?
        .is_some_and(|artist| artist.monitored))
}

async fn matching_catalog_album(
    db: &DatabaseConnection,
    queue: &DownloadQueue,
    artist_id: &str,
    candidate: &TidalAlbum,
) -> Result<Option<album::Model>, DbErr> {
    let identity = AlbumIdentity::from(candidate);
    let catalog_albums = album::Entity::find()
        .inner_join(album_artist::Entity)
        .filter(album_artist::Column::ArtistId.eq(artist_id))
        .order_by_asc(album::Column::Id)
        .all(db)
        .await?;
    let (active_jobs, _) = queue.snapshot().await;
    let matching = catalog_albums
        .into_iter()
        .filter(|album| {
            album.id == candidate.id
                || album.album_type.as_ref().is_some_and(|release_type| {
                    AlbumIdentity::new(&album.title, release_type) == identity
                })
        })
        // Prefer an edition that needs no new job when several equivalent editions are stored.
        .max_by_key(|album| {
            (
                album.relative_path.is_some(),
                active_jobs.iter().any(|job| job.album_id == album.id),
                album.id == candidate.id,
            )
        });
    if matching.is_some() {
        return Ok(matching);
    }
    album::Entity::find_by_id(&candidate.id).one(db).await
}

async fn hand_off_discovery(
    db: &DatabaseConnection,
    tidal: &dyn TidalCatalog,
    queue: &DownloadQueue,
    artist_id: &str,
    candidate: &TidalAlbum,
) -> Result<(), HandoffError> {
    if !is_monitored(db, artist_id).await? {
        return Ok(());
    }
    let catalog_album = match matching_catalog_album(db, queue, artist_id, candidate).await? {
        Some(album) => album,
        None => {
            let prepared = prepare_album(tidal, &candidate.id).await?;
            let transaction = db.begin().await?;
            // Metadata retrieval can outlive a preference change.
            if !is_monitored(&transaction, artist_id).await? {
                return Ok(());
            }
            let outcome = persist_album_in_transaction(&transaction, prepared, None).await?;
            transaction.commit().await?;
            outcome.model
        }
    };
    if catalog_album.relative_path.is_none() && is_monitored(db, artist_id).await? {
        let job = queue.enqueue(catalog_album.id).await?;
        tracing::info!(%artist_id, album_id = %job.album_id, job_id = %job.id, "Handed off monitoring discovery");
    }
    Ok(())
}
