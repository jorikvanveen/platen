use sea_orm::{ActiveValue, ConnectionTrait, DbErr, EntityTrait};

use crate::{
    entity::{album_artist, artist},
    services::tidal::TidalArtist,
};

pub async fn upsert_artist(
    db: &impl ConnectionTrait,
    tidal_artist: &TidalArtist,
) -> Result<(), DbErr> {
    artist::Entity::insert(artist::ActiveModel {
        id: ActiveValue::Set(tidal_artist.id.clone()),
        name: ActiveValue::Set(tidal_artist.name.clone()),
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
mod tests {
    use sea_orm::{ActiveModelTrait, ColumnTrait, Database, QueryFilter, QueryOrder, Set};

    use super::*;
    use crate::entity::album;
    use migration::MigratorTrait;

    // Adding an Album by Tidal ID is the only way rows enter the catalog, so
    // this write path must fit the fresh schema exactly: the retired external
    // identifier columns no longer exist to be written.
    #[tokio::test]
    async fn album_addition_on_fresh_schema_stores_credits_without_retired_fields() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        migration::Migrator::up(&db, None).await.unwrap();

        album::ActiveModel {
            id: Set("tidal-album-1".into()),
            title: Set("Duality".into()),
            album_type: Set(Some("ALBUM".into())),
            release_year: Set(2024),
            release_month: Set(Some(10)),
            release_day: Set(Some(4)),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();

        let tidal_artists = [
            TidalArtist {
                id: "tidal-artist-1".into(),
                name: "BLCKK".into(),
            },
            TidalArtist {
                id: "tidal-artist-2".into(),
                name: "ISSBROKIE".into(),
            },
        ];
        for tidal_artist in &tidal_artists {
            upsert_artist(&db, tidal_artist).await.unwrap();
        }
        insert_credits(&db, "tidal-album-1", &tidal_artists)
            .await
            .unwrap();
        for tidal_artist in &tidal_artists {
            upsert_artist(&db, tidal_artist).await.unwrap();
        }
        insert_credits(&db, "tidal-album-1", &tidal_artists)
            .await
            .unwrap();

        let rows = album_artist::Entity::find()
            .filter(album_artist::Column::AlbumId.eq("tidal-album-1"))
            .find_also_related(artist::Entity)
            .order_by_asc(album_artist::Column::Position)
            .all(&db)
            .await
            .unwrap();
        let credited: Vec<(String, String)> = rows
            .into_iter()
            .map(|(_, artist)| {
                let artist = artist.unwrap();
                (artist.id, artist.name)
            })
            .collect();
        assert_eq!(
            credited,
            [
                ("tidal-artist-1".to_string(), "BLCKK".to_string()),
                ("tidal-artist-2".to_string(), "ISSBROKIE".to_string())
            ]
        );
    }
}
