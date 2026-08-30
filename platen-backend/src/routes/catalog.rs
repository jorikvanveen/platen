use async_trait::async_trait;
use axum::{Json, extract::State};
use reqwest::StatusCode;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, sea_query::Expr};
use tracing::error;
use url::Url;

use crate::{
    AppState,
    entity::{album, artist},
    services::tidal::{Tidal, TidalError},
};

pub mod dto {
    use serde::Serialize;
    use ts_rs::TS;

    #[derive(Debug, Default, PartialEq, Eq, Serialize, TS)]
    #[ts(export)]
    pub struct ArtworkRefreshCounts {
        pub updated: u32,
        pub already_present: u32,
        pub unavailable: u32,
        pub failed: u32,
    }

    #[derive(Debug, Default, PartialEq, Eq, Serialize, TS)]
    #[ts(export)]
    pub struct ArtworkRefreshSummary {
        pub albums: ArtworkRefreshCounts,
        pub artists: ArtworkRefreshCounts,
    }
}

#[async_trait]
trait ArtworkSource {
    async fn album_cover(&self, id: &str) -> Result<Option<String>, TidalError>;
    async fn artist_profile_image(&self, id: &str) -> Result<Option<String>, TidalError>;

    async fn refresh_artwork_with(
        &self,
        db: &DatabaseConnection,
    ) -> Result<dto::ArtworkRefreshSummary, DbErr> {
        let albums = album::Entity::find().all(db).await?;
        let artists = artist::Entity::find().all(db).await?;
        let mut summary = dto::ArtworkRefreshSummary::default();

        for model in albums {
            if model.cover_url.is_some() {
                summary.albums.already_present += 1;
                continue;
            }

            let url = match self.album_cover(&model.id).await {
                Ok(Some(url)) if is_valid_https_url(&url) => url,
                Ok(_) => {
                    summary.albums.unavailable += 1;
                    continue;
                }
                Err(e) => {
                    error!("Could not fetch artwork for album {}: {e:#?}", model.id);
                    summary.albums.failed += 1;
                    continue;
                }
            };

            match album::Entity::update_many()
                .col_expr(album::Column::CoverUrl, Expr::value(url))
                .filter(album::Column::Id.eq(&model.id))
                .filter(album::Column::CoverUrl.is_null())
                .exec(db)
                .await
            {
                Ok(result) if result.rows_affected == 1 => summary.albums.updated += 1,
                Ok(_) => summary.albums.already_present += 1,
                Err(e) => {
                    error!("Could not save artwork for album {}: {e:#?}", model.id);
                    summary.albums.failed += 1;
                }
            }
        }

        for model in artists {
            if model.profile_image_url.is_some() {
                summary.artists.already_present += 1;
                continue;
            }

            let url = match self.artist_profile_image(&model.id).await {
                Ok(Some(url)) if is_valid_https_url(&url) => url,
                Ok(_) => {
                    summary.artists.unavailable += 1;
                    continue;
                }
                Err(e) => {
                    error!("Could not fetch artwork for artist {}: {e:#?}", model.id);
                    summary.artists.failed += 1;
                    continue;
                }
            };

            match artist::Entity::update_many()
                .col_expr(artist::Column::ProfileImageUrl, Expr::value(url))
                .filter(artist::Column::Id.eq(&model.id))
                .filter(artist::Column::ProfileImageUrl.is_null())
                .exec(db)
                .await
            {
                Ok(result) if result.rows_affected == 1 => summary.artists.updated += 1,
                Ok(_) => summary.artists.already_present += 1,
                Err(e) => {
                    error!("Could not save artwork for artist {}: {e:#?}", model.id);
                    summary.artists.failed += 1;
                }
            }
        }

        Ok(summary)
    }
}

#[async_trait]
impl ArtworkSource for Tidal {
    async fn album_cover(&self, id: &str) -> Result<Option<String>, TidalError> {
        self.get_album_cover(id).await
    }

    async fn artist_profile_image(&self, id: &str) -> Result<Option<String>, TidalError> {
        Ok(self.get_artist(id).await?.profile_image_url)
    }
}

pub async fn refresh_artwork(
    State(AppState { tidal, db, .. }): State<AppState>,
) -> Result<Json<dto::ArtworkRefreshSummary>, StatusCode> {
    tidal
        .refresh_artwork_with(&db)
        .await
        .map(Json)
        .map_err(|e| {
            error!("Could not load Catalog records for artwork refresh: {e:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

fn is_valid_https_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .is_some_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use migration::MigratorTrait;
    use sea_orm::{ActiveModelTrait, ConnectionTrait, Database, Set};

    use super::*;

    #[derive(Default)]
    struct TestArtworkSource {
        album_results: HashMap<String, Result<Option<String>, TidalError>>,
        artist_results: HashMap<String, Result<Option<String>, TidalError>>,
    }

    #[async_trait]
    impl ArtworkSource for TestArtworkSource {
        async fn album_cover(&self, id: &str) -> Result<Option<String>, TidalError> {
            self.album_results
                .get(id)
                .map(test_result)
                .unwrap_or(Ok(None))
        }

        async fn artist_profile_image(&self, id: &str) -> Result<Option<String>, TidalError> {
            self.artist_results
                .get(id)
                .map(test_result)
                .unwrap_or(Ok(None))
        }
    }

    fn test_result(
        result: &Result<Option<String>, TidalError>,
    ) -> Result<Option<String>, TidalError> {
        match result {
            Ok(value) => Ok(value.clone()),
            Err(_) => Err(TidalError::UnexpectedResponse),
        }
    }

    async fn test_database() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        migration::Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn insert_album(db: &DatabaseConnection, id: &str, cover_url: Option<&str>) {
        album::ActiveModel {
            id: Set(id.to_owned()),
            title: Set(format!("Album {id}")),
            release_year: Set(2026),
            cover_url: Set(cover_url.map(str::to_owned)),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_artist(db: &DatabaseConnection, id: &str, profile_image_url: Option<&str>) {
        artist::ActiveModel {
            id: Set(id.to_owned()),
            name: Set(format!("Artist {id}")),
            profile_image_url: Set(profile_image_url.map(str::to_owned)),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn refresh_reports_album_and_artist_outcomes_separately() {
        let db = test_database().await;
        insert_album(&db, "album-update", None).await;
        insert_album(&db, "album-present", Some("https://cdn.example/original")).await;
        insert_album(&db, "album-unavailable", None).await;
        insert_album(&db, "album-failed", None).await;
        insert_artist(&db, "artist-update", None).await;
        insert_artist(&db, "artist-present", Some("https://cdn.example/original")).await;
        insert_artist(&db, "artist-unavailable", None).await;
        insert_artist(&db, "artist-failed", None).await;

        let source = TestArtworkSource {
            album_results: HashMap::from([
                (
                    "album-update".into(),
                    Ok(Some("https://cdn.example/album".into())),
                ),
                ("album-unavailable".into(), Ok(None)),
                ("album-failed".into(), Err(TidalError::UnexpectedResponse)),
            ]),
            artist_results: HashMap::from([
                (
                    "artist-update".into(),
                    Ok(Some("https://cdn.example/artist".into())),
                ),
                ("artist-unavailable".into(), Ok(None)),
                ("artist-failed".into(), Err(TidalError::UnexpectedResponse)),
            ]),
        };

        let summary = source.refresh_artwork_with(&db).await.unwrap();

        assert_eq!(
            summary.albums,
            dto::ArtworkRefreshCounts {
                updated: 1,
                already_present: 1,
                unavailable: 1,
                failed: 1,
            }
        );
        assert_eq!(
            summary.artists,
            dto::ArtworkRefreshCounts {
                updated: 1,
                already_present: 1,
                unavailable: 1,
                failed: 1,
            }
        );
        assert_eq!(
            album::Entity::find_by_id("album-update")
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .cover_url
                .as_deref(),
            Some("https://cdn.example/album")
        );
        assert_eq!(
            artist::Entity::find_by_id("artist-update")
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .profile_image_url
                .as_deref(),
            Some("https://cdn.example/artist")
        );
    }

    #[tokio::test]
    async fn persistence_failure_does_not_stop_later_records() {
        let db = test_database().await;
        insert_album(&db, "album-fail-save", None).await;
        insert_album(&db, "album-update", None).await;
        db.execute_unprepared(
            "CREATE TRIGGER fail_artwork_update \
             BEFORE UPDATE OF cover_url ON album \
             WHEN NEW.id = 'album-fail-save' \
             BEGIN SELECT RAISE(FAIL, 'test persistence failure'); END",
        )
        .await
        .unwrap();
        let source = TestArtworkSource {
            album_results: HashMap::from([
                (
                    "album-fail-save".into(),
                    Ok(Some("https://cdn.example/failure".into())),
                ),
                (
                    "album-update".into(),
                    Ok(Some("https://cdn.example/updated".into())),
                ),
            ]),
            ..Default::default()
        };

        let summary = source.refresh_artwork_with(&db).await.unwrap();

        assert_eq!(summary.albums.failed, 1);
        assert_eq!(summary.albums.updated, 1);
        assert_eq!(
            album::Entity::find_by_id("album-update")
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .cover_url
                .as_deref(),
            Some("https://cdn.example/updated")
        );
    }

    #[tokio::test]
    async fn refresh_rejects_invalid_urls_without_persisting_them() {
        let db = test_database().await;
        insert_album(&db, "album-http", None).await;
        insert_artist(&db, "artist-relative", None).await;
        let source = TestArtworkSource {
            album_results: HashMap::from([(
                "album-http".into(),
                Ok(Some("http://cdn.example/album".into())),
            )]),
            artist_results: HashMap::from([("artist-relative".into(), Ok(Some("/artist".into())))]),
        };

        let summary = source.refresh_artwork_with(&db).await.unwrap();

        assert_eq!(summary.albums.unavailable, 1);
        assert_eq!(summary.artists.unavailable, 1);
        assert!(
            album::Entity::find_by_id("album-http")
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .cover_url
                .is_none()
        );
        assert!(
            artist::Entity::find_by_id("artist-relative")
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .profile_image_url
                .is_none()
        );
    }
}
