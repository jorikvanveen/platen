use std::collections::HashSet;

use sea_orm::{
    ActiveValue, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, TransactionError,
    TransactionTrait, TryInsertResult,
};

use crate::{
    entity::{album, album_artist, artist},
    routes::album::{ReleaseDate, parse_release_date},
    services::tidal::{
        ResolvedTidalSearchedAlbum, TidalAlbum, TidalArtist, TidalCatalog, TidalError,
    },
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum PrepareAlbumError {
    #[error(transparent)]
    Tidal(#[from] TidalError),
    #[error("Album has a missing or invalid release date")]
    InvalidReleaseDate,
    #[error("Album credits must be nonempty with unique, nonblank artist IDs and nonblank names")]
    InvalidCredits,
    #[error("Tidal returned an album ID different from the requested ID")]
    AlbumIdMismatch,
}

#[derive(Debug)]
pub(crate) struct PreparedAlbum {
    album: TidalAlbum,
    artists: Vec<TidalArtist>,
    release_date: ReleaseDate,
}

impl PreparedAlbum {
    fn new(album: TidalAlbum, artists: Vec<TidalArtist>) -> Result<Self, PrepareAlbumError> {
        let release_date = album
            .release_date
            .as_deref()
            .and_then(|date| parse_release_date(date).ok())
            .ok_or(PrepareAlbumError::InvalidReleaseDate)?;
        let mut artist_ids = HashSet::new();
        if artists.is_empty()
            || artists.len() > i32::MAX as usize
            || artists.iter().any(|artist| {
                artist.id.trim().is_empty()
                    || artist.name.trim().is_empty()
                    || !artist_ids.insert(artist.id.as_str())
            })
        {
            return Err(PrepareAlbumError::InvalidCredits);
        }
        Ok(Self {
            album,
            artists,
            release_date,
        })
    }

    pub(crate) fn album(&self) -> &TidalAlbum {
        &self.album
    }
}

impl TryFrom<ResolvedTidalSearchedAlbum> for PreparedAlbum {
    type Error = PrepareAlbumError;

    fn try_from(hit: ResolvedTidalSearchedAlbum) -> Result<Self, Self::Error> {
        Self::new(
            TidalAlbum {
                id: hit.id,
                title: hit.title,
                cover_url: hit.cover_url,
                release_date: hit.release_date,
                popularity: hit.popularity,
                r#type: hit.r#type,
                explicit: hit.explicit,
                media_tags: hit.media_tags,
            },
            hit.artists,
        )
    }
}

pub(crate) async fn prepare_album(
    source: &dyn TidalCatalog,
    id: &str,
) -> Result<PreparedAlbum, PrepareAlbumError> {
    let mut album = source.get_album(id).await?;
    if album.id != id {
        return Err(PrepareAlbumError::AlbumIdMismatch);
    }
    let artists = source.get_album_artists(id).await?;
    album.cover_url = source.get_album_cover(id).await?;
    PreparedAlbum::new(album, artists)
}

#[derive(Debug)]
pub(crate) struct PersistAlbumOutcome {
    pub(crate) model: album::Model,
    pub(crate) imported: bool,
}

pub(crate) async fn persist_album(
    db: &DatabaseConnection,
    prepared: PreparedAlbum,
    relative_path: Option<String>,
) -> Result<PersistAlbumOutcome, TransactionError<DbErr>> {
    db.transaction::<_, PersistAlbumOutcome, DbErr>(|transaction| {
        Box::pin(async move {
            let PreparedAlbum {
                album,
                artists,
                release_date,
            } = prepared;
            let album_id = album.id.clone();
            let inserted = album::Entity::insert(album::ActiveModel {
                id: ActiveValue::Set(album.id),
                title: ActiveValue::Set(album.title),
                cover_url: ActiveValue::Set(album.cover_url),
                album_type: ActiveValue::Set(Some(album.r#type)),
                release_year: ActiveValue::Set(release_date.year),
                release_month: ActiveValue::Set(release_date.month),
                release_day: ActiveValue::Set(release_date.day),
                relative_path: ActiveValue::Set(relative_path),
                explicit: ActiveValue::Set(album.explicit),
                media_tags: ActiveValue::Set(album.media_tags.map(serde_json::Value::from)),
            })
            .on_conflict_do_nothing()
            .exec(transaction)
            .await?;
            let imported = matches!(inserted, TryInsertResult::Inserted(_));
            // A concurrent creator may win the insert. Its metadata and credits must stay intact.
            if imported {
                for artist in &artists {
                    upsert_artist(transaction, artist).await?;
                }
                insert_credits(transaction, &album_id, &artists).await?;
            }
            let model = album::Entity::find_by_id(&album_id)
                .one(transaction)
                .await?
                .ok_or_else(|| {
                    DbErr::RecordNotFound(format!("Album {album_id} was not found after insertion"))
                })?;
            Ok(PersistAlbumOutcome { model, imported })
        })
    })
    .await
}

pub async fn upsert_artist(
    db: &impl ConnectionTrait,
    tidal_artist: &TidalArtist,
) -> Result<(), DbErr> {
    artist::Entity::insert(artist::ActiveModel {
        id: ActiveValue::Set(tidal_artist.id.clone()),
        name: ActiveValue::Set(tidal_artist.name.clone()),
        profile_image_url: ActiveValue::Set(tidal_artist.profile_image_url.clone()),
    })
    .on_conflict_do_nothing()
    .exec(db)
    .await?;

    Ok(())
}

pub async fn insert_credits(
    db: &impl ConnectionTrait,
    album_id: &str,
    tidal_artists: &[TidalArtist],
) -> Result<(), DbErr> {
    for (position, tidal_artist) in tidal_artists.iter().enumerate() {
        album_artist::Entity::insert(album_artist::ActiveModel {
            album_id: ActiveValue::Set(album_id.to_string()),
            artist_id: ActiveValue::Set(tidal_artist.id.clone()),
            position: ActiveValue::Set(position as i32),
        })
        .on_conflict_do_nothing()
        .exec(db)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/services/catalog/utils.rs"]
mod tests;
