
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter,
};

use crate::{
    entity::{album_artist, artist},
    services::tidal::TidalArtist,
};

pub async fn upsert_artist(
    db: &impl ConnectionTrait,
    tidal_artist: &TidalArtist,
) -> Result<(), DbErr> {
    if artist::Entity::find_by_id(&tidal_artist.id)
        .one(db)
        .await?
        .is_some()
    {
        return Ok(());
    }

    artist::ActiveModel {
        id: ActiveValue::Set(tidal_artist.id.clone()),
        name: ActiveValue::Set(tidal_artist.name.clone()),
        musicbrainz_artist_id: ActiveValue::Set(None),
    }
    .insert(db)
    .await?;
    Ok(())
}

pub async fn insert_credits(
    db: &impl ConnectionTrait,
    album_id: &str,
    tidal_artists: &[TidalArtist],
) -> Result<(), DbErr> {
    for (position, tidal_artist) in tidal_artists.iter().enumerate() {
        let existing = album_artist::Entity::find()
            .filter(album_artist::Column::AlbumId.eq(album_id))
            .filter(album_artist::Column::ArtistId.eq(&tidal_artist.id))
            .one(db)
            .await?;
        if existing.is_some() {
            continue;
        }

        album_artist::ActiveModel {
            album_id: ActiveValue::Set(album_id.to_string()),
            artist_id: ActiveValue::Set(tidal_artist.id.clone()),
            position: ActiveValue::Set(position as i32),
        }
        .insert(db)
        .await?;
    }
    Ok(())
}

