use std::{collections::HashSet, sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QuerySelect, Set,
    TransactionTrait, sea_query::Expr,
};
use tokio::{task::JoinHandle, time::MissedTickBehavior};

use super::{discovery::AlbumIdentity, tidal::TidalCatalog};
use crate::entity::{artist, artist_known_album};

const CHECK_INTERVAL: TimeDelta = TimeDelta::hours(6);

pub(crate) fn start(db: DatabaseConnection, tidal: Arc<dyn TidalCatalog>) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Sweeping the Catalog also finds Artists introduced by scans and future import paths.
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = initialize_due_artists(&db, tidal.as_ref(), Utc::now()).await {
                tracing::error!(%error, "Could not find Artists needing a Monitoring Baseline");
            }
        }
    })
}

pub(crate) async fn initialize_due_artists(
    db: &DatabaseConnection,
    tidal: &dyn TidalCatalog,
    now: DateTime<Utc>,
) -> Result<(), DbErr> {
    let latest_due_attempt_at = now
        .checked_sub_signed(CHECK_INTERVAL)
        .ok_or_else(|| DbErr::Custom("Artist check time exceeds the supported range".to_owned()))?;
    let artist_ids = artist::Entity::find()
        .select_only()
        .column(artist::Column::Id)
        .filter(artist::Column::MonitoringBaselineInitializedAt.is_null())
        .filter(
            Condition::any()
                .add(artist::Column::LastCheckAttemptAt.is_null())
                .add(artist::Column::LastCheckAttemptAt.lte(latest_due_attempt_at)),
        )
        .into_tuple::<String>()
        .all(db)
        .await?;
    for artist_id in artist_ids {
        if let Err(error) = initialize_monitoring_baseline(db, tidal, &artist_id, now).await {
            tracing::error!(%artist_id, %error, "Could not persist Artist Monitoring Baseline");
        }
    }
    Ok(())
}

async fn initialize_monitoring_baseline(
    db: &DatabaseConnection,
    tidal: &dyn TidalCatalog,
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
            tracing::warn!(%artist_id, %error, "Monitoring Baseline fetch failed; will retry");
            return Ok(());
        }
    };
    let identities: HashSet<_> = albums.iter().map(AlbumIdentity::from).collect();
    let transaction = db.begin().await?;
    artist::Entity::update_many()
        .col_expr(
            artist::Column::MonitoringBaselineInitializedAt,
            Expr::value(now),
        )
        .filter(artist::Column::Id.eq(artist_id))
        .exec(&transaction)
        .await?;
    for identity in &identities {
        artist_known_album::Entity::insert(artist_known_album::ActiveModel {
            artist_id: Set(artist_id.to_owned()),
            normalized_title: Set(identity.normalized_title.clone()),
            release_type: Set(identity.release_type.clone()),
        })
        .exec(&transaction)
        .await?;
    }
    transaction.commit().await?;
    tracing::info!(%artist_id, entries = identities.len(), "Initialized Artist Monitoring Baseline");
    Ok(())
}
