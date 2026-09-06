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

    #[cfg(test)]
    pub(crate) fn artists(&self) -> &[TidalArtist] {
        &self.artists
    }

    #[cfg(test)]
    pub(crate) fn release_date(&self) -> ReleaseDate {
        self.release_date
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
mod tests {
    use sea_orm::{ActiveModelTrait, ColumnTrait, Database, QueryFilter, QueryOrder, Set};

    use super::*;
    use crate::entity::album;
    use crate::services::tidal::ResolvedTidalSearchedAlbum;
    use migration::MigratorTrait;

    struct FakeCatalog {
        album: TidalAlbum,
        artists: Vec<TidalArtist>,
        cover_fails: bool,
        metadata_fails: bool,
        artists_fail: bool,
    }

    impl Default for FakeCatalog {
        fn default() -> Self {
            Self {
                album: TidalAlbum {
                    id: "album-1".into(),
                    title: "Shared album".into(),
                    cover_url: None,
                    release_date: Some("2024-02-29".into()),
                    popularity: 0.0,
                    r#type: "ALBUM".into(),
                },
                artists: vec![
                    TidalArtist {
                        id: "artist-2".into(),
                        name: "Primary".into(),
                        profile_image_url: None,
                    },
                    TidalArtist {
                        id: "artist-1".into(),
                        name: "Guest".into(),
                        profile_image_url: None,
                    },
                ],
                cover_fails: false,
                metadata_fails: false,
                artists_fail: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl TidalCatalog for FakeCatalog {
        async fn find_album(&self, _: &str) -> Result<Vec<ResolvedTidalSearchedAlbum>, TidalError> {
            Ok(Vec::new())
        }

        async fn get_album(&self, _: &str) -> Result<TidalAlbum, TidalError> {
            if self.metadata_fails {
                return Err(TidalError::UnexpectedResponse);
            }
            Ok(self.album.clone())
        }

        async fn get_album_cover(&self, _: &str) -> Result<Option<String>, TidalError> {
            if self.cover_fails {
                return Err(TidalError::UnexpectedResponse);
            }
            Ok(self.album.cover_url.clone())
        }

        async fn get_album_artists(&self, _: &str) -> Result<Vec<TidalArtist>, TidalError> {
            if self.artists_fail {
                return Err(TidalError::UnexpectedResponse);
            }
            Ok(self.artists.clone())
        }
    }

    async fn test_database() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        migration::Migrator::up(&db, None).await.unwrap();
        db
    }

    #[tokio::test]
    async fn preparation_tolerates_missing_cover_and_preserves_credit_order() {
        let source = FakeCatalog::default();
        let prepared = prepare_album(&source, "album-1").await.unwrap();
        assert_eq!(prepared.album().title, "Shared album");
        assert_eq!(prepared.album().cover_url, None);
        assert_eq!(
            prepared.release_date(),
            ReleaseDate {
                year: 2024,
                month: Some(2),
                day: Some(29)
            }
        );
        assert_eq!(
            prepared
                .artists()
                .iter()
                .map(|artist| artist.id.as_str())
                .collect::<Vec<_>>(),
            ["artist-2", "artist-1"]
        );
    }

    #[tokio::test]
    async fn preparation_propagates_cover_fetch_error() {
        let source = FakeCatalog {
            cover_fails: true,
            ..Default::default()
        };
        assert!(matches!(
            prepare_album(&source, "album-1").await,
            Err(PrepareAlbumError::Tidal(TidalError::UnexpectedResponse))
        ));
    }

    #[tokio::test]
    async fn preparation_rejects_missing_and_invalid_dates() {
        for date in [
            None,
            Some(""),
            Some("2023-02-29"),
            Some("2024-13"),
            Some("0000"),
        ] {
            let mut source = FakeCatalog::default();
            source.album.release_date = date.map(str::to_owned);
            assert!(matches!(
                prepare_album(&source, "album-1").await,
                Err(PrepareAlbumError::InvalidReleaseDate)
            ));
        }
        for date in ["2024", "2024-02", "2024-02-29"] {
            let mut source = FakeCatalog::default();
            source.album.release_date = Some(date.into());
            assert!(prepare_album(&source, "album-1").await.is_ok());
        }
    }

    #[tokio::test]
    async fn preparation_rejects_empty_duplicate_and_blank_credits() {
        let valid_artist = FakeCatalog::default().artists.remove(0);
        for artists in [
            vec![],
            vec![valid_artist.clone(), valid_artist.clone()],
            vec![TidalArtist {
                id: "  ".into(),
                ..valid_artist.clone()
            }],
            vec![TidalArtist {
                name: "\t".into(),
                ..valid_artist
            }],
        ] {
            let source = FakeCatalog {
                artists,
                ..Default::default()
            };
            assert!(matches!(
                prepare_album(&source, "album-1").await,
                Err(PrepareAlbumError::InvalidCredits)
            ));
        }
    }

    #[tokio::test]
    async fn preparation_rejects_wrong_album_and_propagates_required_fetch_errors() {
        assert!(matches!(
            prepare_album(&FakeCatalog::default(), "other-id").await,
            Err(PrepareAlbumError::AlbumIdMismatch)
        ));
        for source in [
            FakeCatalog {
                metadata_fails: true,
                ..Default::default()
            },
            FakeCatalog {
                artists_fail: true,
                ..Default::default()
            },
        ] {
            assert!(matches!(
                prepare_album(&source, "album-1").await,
                Err(PrepareAlbumError::Tidal(_))
            ));
        }
    }

    #[tokio::test]
    async fn persistence_stores_location_cover_and_ordered_credits_without_refreshing_existing_album()
     {
        let db = test_database().await;
        let mut source = FakeCatalog::default();
        source.album.cover_url = Some("https://example.test/cover".into());
        let prepared = prepare_album(&source, "album-1").await.unwrap();
        let outcome = persist_album(&db, prepared, Some("Primary/Shared album".into()))
            .await
            .unwrap();
        assert!(outcome.imported);
        assert_eq!(
            outcome.model.relative_path.as_deref(),
            Some("Primary/Shared album")
        );
        assert_eq!(outcome.model.cover_url, source.album.cover_url);
        let credits = album_artist::Entity::find()
            .order_by_asc(album_artist::Column::Position)
            .all(&db)
            .await
            .unwrap();
        assert_eq!(
            credits
                .iter()
                .map(|credit| (credit.artist_id.as_str(), credit.position))
                .collect::<Vec<_>>(),
            [("artist-2", 0), ("artist-1", 1)]
        );

        source.album.title = "Replacement".into();
        source.artists = vec![TidalArtist {
            id: "replacement".into(),
            name: "Replacement".into(),
            profile_image_url: None,
        }];
        for path in [None, Some("Replacement/path".into())] {
            let prepared = prepare_album(&source, "album-1").await.unwrap();
            let repeated = persist_album(&db, prepared, path).await.unwrap();
            assert!(!repeated.imported);
            assert_eq!(repeated.model, outcome.model);
        }
        assert_eq!(
            album_artist::Entity::find()
                .order_by_asc(album_artist::Column::Position)
                .all(&db)
                .await
                .unwrap(),
            credits
        );
        assert!(
            artist::Entity::find_by_id("replacement")
                .one(&db)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn persistence_without_location_keeps_album_undownloaded() {
        let db = test_database().await;
        let prepared = prepare_album(&FakeCatalog::default(), "album-1")
            .await
            .unwrap();
        let outcome = persist_album(&db, prepared, None).await.unwrap();
        assert!(outcome.imported);
        assert_eq!(outcome.model.relative_path, None);
    }

    #[tokio::test]
    async fn failed_credit_insert_rolls_back_album_artists_credits_and_location() {
        let db = test_database().await;
        db.execute_unprepared("CREATE TRIGGER reject_guest BEFORE INSERT ON album_artist WHEN NEW.position = 1 BEGIN SELECT RAISE(ABORT, 'test credit failure'); END").await.unwrap();
        let prepared = prepare_album(&FakeCatalog::default(), "album-1")
            .await
            .unwrap();
        assert!(
            persist_album(&db, prepared, Some("Primary/Shared album".into()))
                .await
                .is_err()
        );
        assert!(album::Entity::find().all(&db).await.unwrap().is_empty());
        assert!(artist::Entity::find().all(&db).await.unwrap().is_empty());
        assert!(
            album_artist::Entity::find()
                .all(&db)
                .await
                .unwrap()
                .is_empty()
        );
    }

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
                profile_image_url: Some("https://cdn.example/blckk".into()),
            },
            TidalArtist {
                id: "tidal-artist-2".into(),
                name: "ISSBROKIE".into(),
                profile_image_url: None,
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

    #[tokio::test]
    async fn upsert_artist_does_not_overwrite_existing_profile_image() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        migration::Migrator::up(&db, None).await.unwrap();

        let artist = TidalArtist {
            id: "tidal-artist-1".into(),
            name: "BLCKK".into(),
            profile_image_url: Some("https://cdn.example/original".into()),
        };
        upsert_artist(&db, &artist).await.unwrap();

        let replacement_image = TidalArtist {
            profile_image_url: Some("https://cdn.example/replacement".into()),
            ..artist
        };
        upsert_artist(&db, &replacement_image).await.unwrap();

        let stored_artist = artist::Entity::find_by_id("tidal-artist-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored_artist.profile_image_url.as_deref(),
            Some("https://cdn.example/original")
        );
    }
}
