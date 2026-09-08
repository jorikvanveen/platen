use std::collections::HashMap;

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    TransactionError, TransactionTrait, sea_query::Expr,
};

use super::model::{CatalogAlbum, ImportOutcome, LocationOutcome, LocationUpdate};
use crate::{
    entity::{album, album_artist, artist},
    services::catalog::{PreparedAlbum, persist_album_in_transaction},
};

#[derive(Clone)]
pub(super) struct ImportRepository {
    db: DatabaseConnection,
}

impl ImportRepository {
    pub(super) fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub(super) async fn albums(&self) -> Result<Vec<CatalogAlbum>, TransactionError<DbErr>> {
        self.db
            .transaction(|transaction| {
                Box::pin(async move {
                    let albums = album::Entity::find().all(transaction).await?;
                    let credits = album_artist::Entity::find()
                        .find_both_related(artist::Entity)
                        .order_by_asc(album_artist::Column::Position)
                        .all(transaction)
                        .await?;
                    let mut primary_artists_by_album_id = HashMap::new();
                    for (credit, artist) in credits {
                        primary_artists_by_album_id
                            .entry(credit.album_id)
                            .or_insert(artist.name);
                    }
                    Ok(albums
                        .into_iter()
                        .map(|album| catalog_album(album, &mut primary_artists_by_album_id))
                        .collect())
                })
            })
            .await
    }

    pub(super) async fn location(&self, album_id: &str) -> Result<Option<String>, DbErr> {
        Ok(album::Entity::find_by_id(album_id)
            .one(&self.db)
            .await?
            .and_then(|album| album.relative_path))
    }

    pub(super) async fn apply_location(
        &self,
        update: LocationUpdate,
    ) -> Result<LocationOutcome, TransactionError<DbErr>> {
        self.db
            .transaction(|transaction| {
                Box::pin(async move {
                    if let Some(path) = &update.relative_path
                        && path_is_occupied(transaction, &update.album_id, path).await?
                    {
                        return Err(DbErr::Custom(format!("Album location {path} is occupied")));
                    }
                    let mut query = album::Entity::update_many()
                        .col_expr(
                            album::Column::RelativePath,
                            Expr::value(update.relative_path.clone()),
                        )
                        .filter(album::Column::Id.eq(&update.album_id));
                    query = match &update.previous_path {
                        Some(path) => query.filter(album::Column::RelativePath.eq(path)),
                        None => query.filter(album::Column::RelativePath.is_null()),
                    };
                    if query.exec(transaction).await?.rows_affected != 1 {
                        return Err(DbErr::Custom(format!(
                            "Album {} was deleted or its location changed during the scan",
                            update.album_id
                        )));
                    }
                    Ok(match (&update.previous_path, &update.relative_path) {
                        (None, Some(_)) => LocationOutcome::Attached,
                        (Some(_), Some(_)) => LocationOutcome::Changed,
                        (_, None) => LocationOutcome::Cleared,
                    })
                })
            })
            .await
    }

    pub(super) async fn import(
        &self,
        prepared: PreparedAlbum,
        relative_path: String,
    ) -> Result<ImportOutcome, TransactionError<DbErr>> {
        self.db
            .transaction(|transaction| {
                Box::pin(async move {
                    let album_id = prepared.album().id.clone();
                    if path_is_occupied(transaction, &album_id, &relative_path).await? {
                        return Ok(ImportOutcome::Duplicate { stored_path: None });
                    }
                    let outcome = persist_album_in_transaction(
                        transaction,
                        prepared,
                        Some(relative_path.clone()),
                    )
                    .await?;
                    if outcome.imported {
                        return Ok(ImportOutcome::Imported);
                    }
                    match outcome.model.relative_path {
                        Some(path) if path == relative_path => {
                            Ok(ImportOutcome::Location(LocationOutcome::Unchanged))
                        }
                        Some(path) => Ok(ImportOutcome::Duplicate {
                            stored_path: Some(path),
                        }),
                        None => {
                            // Manual creation may have inserted this Album without a location during matching.
                            let updated = album::Entity::update_many()
                                .col_expr(album::Column::RelativePath, Expr::value(relative_path))
                                .filter(album::Column::Id.eq(&album_id))
                                .filter(album::Column::RelativePath.is_null())
                                .exec(transaction)
                                .await?;
                            if updated.rows_affected != 1 {
                                return Err(DbErr::Custom(format!(
                                    "Album {album_id} changed during import"
                                )));
                            }
                            Ok(ImportOutcome::Location(LocationOutcome::Attached))
                        }
                    }
                })
            })
            .await
    }
}

fn catalog_album(
    album: album::Model,
    primary_artists_by_album_id: &mut HashMap<String, String>,
) -> CatalogAlbum {
    CatalogAlbum {
        primary_artist: primary_artists_by_album_id.remove(&album.id),
        id: album.id,
        title: album.title,
        release_year: album.release_year,
        relative_path: album.relative_path,
    }
}

async fn path_is_occupied(
    db: &impl ConnectionTrait,
    album_id: &str,
    path: &str,
) -> Result<bool, DbErr> {
    Ok(album::Entity::find()
        .filter(album::Column::RelativePath.eq(path))
        .filter(album::Column::Id.ne(album_id))
        .one(db)
        .await?
        .is_some())
}

#[cfg(test)]
#[path = "../../tests/services/import/repository.rs"]
mod tests;
