use axum::{Json, extract::State};
use reqwest::StatusCode;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use tracing::{error, info, warn};

use crate::{
    AppState,
    entity::{album, artist},
    routes::album::{ReleaseDate, parse_release_date},
    services::{catalog_utils, import::{Failure as ServiceFailure, Status as ServiceStatus, Summary as ServiceSummary}, musicbrainz::RequestError as MbRequestError, tidal::TidalArtist},
};

pub mod dto {
    use serde::Serialize;
    use ts_rs::TS;

    #[derive(Debug, Clone, Serialize, TS)]
    #[ts(export)]
    pub struct ImportFailure {
        pub name: String,
        pub reason: String,
    }

    #[derive(Debug, Clone, Serialize, TS)]
    #[ts(export)]
    pub struct ImportSummary {
        pub total_scanned: u32,
        pub created: u32,
        pub linked: u32,
        pub skipped: u32,
        pub failed: u32,
        pub failures: Vec<ImportFailure>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
    #[serde(rename_all = "lowercase")]
    #[ts(export)]
    pub enum ImportStateKind {
        Idle,
        Running,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct ImportStatus {
        pub state: ImportStateKind,
        pub last_summary: Option<ImportSummary>,
    }
}

/// Error response for [`import`]. `Conflict` carries the typed `ImportStatus`
/// body; `BadGateway` keeps the empty-body `502` clients already handle.
pub(crate) enum ImportError {
    Conflict(dto::ImportStatus),
    BadGateway,
}

impl axum::response::IntoResponse for ImportError {
    fn into_response(self) -> axum::response::Response {
        match self {
            ImportError::Conflict(status) => (StatusCode::CONFLICT, Json(status)).into_response(),
            ImportError::BadGateway => StatusCode::BAD_GATEWAY.into_response(),
        }
    }
}

impl From<ServiceFailure> for dto::ImportFailure {
    fn from(f: ServiceFailure) -> Self {
        dto::ImportFailure {
            name: f.name,
            reason: f.reason,
        }
    }
}

/// Map a service-side [`ServiceSummary`] onto the wire DTO. The service layer
/// stays serde-free; serialization concerns live here.
impl From<ServiceSummary> for dto::ImportSummary {
    fn from(s: ServiceSummary) -> Self {
        dto::ImportSummary {
            total_scanned: s.total_scanned,
            created: s.created,
            linked: s.linked,
            skipped: s.skipped,
            failed: s.failed,
            failures: s.failures.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ServiceStatus> for dto::ImportStatus {
    fn from(s: ServiceStatus) -> Self {
        dto::ImportStatus {
            state: if s.running {
                dto::ImportStateKind::Running
            } else {
                dto::ImportStateKind::Idle
            },
            last_summary: s.last_summary.map(Into::into),
        }
    }
}

#[axum::debug_handler]
pub async fn import(
    State(AppState {
        import,
        musicbrainz,
        jellyfin,
        tidal,
        db,
        ..
    }): State<AppState>,
) -> Result<Json<dto::ImportSummary>, ImportError> {
    info!("Starting Jellyfin import");

    let mut guard = match import.try_begin_import().await {
        Ok(g) => g,
        Err(status) => {
            info!("Rejecting concurrent Jellyfin import");
            return Err(ImportError::Conflict(status.into()));
        }
    };

    let jellyfin_albums = jellyfin.list_albums().await.map_err(|e| {
        error!("Jellyfin list_albums failed: {e:#?}");
        ImportError::BadGateway
    })?;

    let mut summary = ServiceSummary {
        total_scanned: jellyfin_albums.len() as u32,
        created: 0,
        linked: 0,
        skipped: 0,
        failed: 0,
        failures: Vec::new(),
    };

    for jf_album in jellyfin_albums {
        let mb_release_group_id = jf_album
            .provider_ids
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("musicbrainzreleasegroup"))
            .map(|(_, v)| v.clone());
        let mb_artist_id = jf_album
            .provider_ids
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("musicbrainzartist"))
            .map(|(_, v)| v.clone());

        let Some(mb_release_group_id) = mb_release_group_id else {
            summary.skipped += 1;
            continue;
        };

        match resolve_album(
            &db,
            &musicbrainz,
            &tidal,
            &jf_album,
            mb_release_group_id,
            mb_artist_id,
        )
        .await
        {
            Ok(Outcome::Linked) => summary.linked += 1,
            Ok(Outcome::Created) => summary.created += 1,
            Ok(Outcome::Skipped) => summary.skipped += 1,
            Err(reason) => {
                summary.failed += 1;
                summary.failures.push(ServiceFailure {
                    name: jf_album.name,
                    reason,
                });
            }
        }
    }

    guard.finish(summary.clone()).await;
    Ok(Json(summary.into()))
}

/// `GET /jellyfin/import/status`: always `200` with the current
/// [`dto::ImportStatus`].
#[axum::debug_handler]
pub async fn status(State(AppState { import, .. }): State<AppState>) -> Json<dto::ImportStatus> {
    let status = import.status().await;
    Json(status.into())
}

enum Outcome {
    Linked,
    Created,
    Skipped,
}

// `Link` dwarfs the other variants, but the value lives only until the
// caller's match consumes it.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
enum ExistingMbidDecision {
    Skip,
    Link {
        existing: album::Model,
        jellyfin_id: String,
    },
    Proceed,
}

#[derive(Debug, PartialEq)]
enum ArtistDecision {
    LinkMusicBrainz {
        existing: artist::Model,
        mb_artist_id: String,
    },
    Insert {
        id: String,
        name: String,
        mb_artist_id: Option<String>,
    },
    Noop,
}

#[derive(Debug, PartialEq)]
enum AlbumDecision {
    LinkExisting {
        existing: album::Model,
        jellyfin_id: Option<String>,
        musicbrainz_release_group_id: Option<String>,
    },
    Create {
        id: String,
        title: String,
        album_type: Option<String>,
        jellyfin_id: String,
        musicbrainz_release_group_id: String,
        release_date: ReleaseDate,
    },
}

fn decide_existing_mbid(
    existing_by_mbid: Option<album::Model>,
    jf_album: &crate::services::jellyfin::JellyfinAlbum,
) -> ExistingMbidDecision {
    match existing_by_mbid {
        Some(existing) if existing.jellyfin_id.is_some() => ExistingMbidDecision::Skip,
        Some(existing) => ExistingMbidDecision::Link {
            existing,
            jellyfin_id: jf_album.id.clone(),
        },
        None => ExistingMbidDecision::Proceed,
    }
}

fn decide_artist(
    existing_artist: Option<artist::Model>,
    tidal_artist: &crate::services::tidal::TidalArtist,
    mb_artist_id: Option<&str>,
) -> ArtistDecision {
    match existing_artist {
        Some(existing) if existing.musicbrainz_artist_id.is_none() => {
            if let Some(mb_id) = mb_artist_id {
                ArtistDecision::LinkMusicBrainz {
                    existing,
                    mb_artist_id: mb_id.to_string(),
                }
            } else {
                ArtistDecision::Noop
            }
        }
        Some(_) => ArtistDecision::Noop,
        None => ArtistDecision::Insert {
            id: tidal_artist.id.clone(),
            name: tidal_artist.name.clone(),
            mb_artist_id: mb_artist_id.map(str::to_string),
        },
    }
}

fn decide_album_insert(
    existing_by_tidal_id: Option<album::Model>,
    title: &str,
    tidal_hit: &crate::services::tidal::ResolvedTidalSearchedAlbum,
    jf_album: &crate::services::jellyfin::JellyfinAlbum,
    mb_release_group_id: &str,
    release_date: Option<ReleaseDate>,
) -> AlbumDecision {
    match existing_by_tidal_id {
        Some(existing) => {
            let jellyfin_id = if existing.jellyfin_id.is_none() {
                Some(jf_album.id.clone())
            } else {
                existing.jellyfin_id.clone()
            };
            let musicbrainz_release_group_id = if existing.musicbrainz_release_group_id.is_none() {
                Some(mb_release_group_id.to_string())
            } else {
                existing.musicbrainz_release_group_id.clone()
            };
            AlbumDecision::LinkExisting {
                existing,
                jellyfin_id,
                musicbrainz_release_group_id,
            }
        }
        None => AlbumDecision::Create {
            id: tidal_hit.id.clone(),
            title: title.to_string(),
            album_type: Some(tidal_hit.r#type.clone()),
            jellyfin_id: jf_album.id.clone(),
            musicbrainz_release_group_id: mb_release_group_id.to_string(),
            // Invariant: the caller (resolve_album) computes release_date = Some(...)
            // iff existing_by_tidal_id is None, so the Create arm always has a date.
            release_date: release_date.expect("release date required for new album"),
        },
    }
}

/// Resolve a single Jellyfin album to a Tidal-keyed `album` row.
async fn resolve_album(
    db: &sea_orm::DatabaseConnection,
    musicbrainz: &crate::services::musicbrainz::Musicbrainz,
    tidal: &crate::services::tidal::Tidal,
    jf_album: &crate::services::jellyfin::JellyfinAlbum,
    mb_release_group_id: String,
    mb_artist_id: Option<String>,
) -> Result<Outcome, String> {
    let existing_by_mbid = album::Entity::find()
        .filter(album::Column::MusicbrainzReleaseGroupId.eq(mb_release_group_id.clone()))
        .one(db)
        .await
        .map_err(|e| {
            error!("Db error looking up album by MBID: {e:#?}");
            "database error".to_string()
        })?;

    match decide_existing_mbid(existing_by_mbid, jf_album) {
        ExistingMbidDecision::Skip => return Ok(Outcome::Skipped),
        ExistingMbidDecision::Link {
            existing,
            jellyfin_id,
        } => {
            let mut active: album::ActiveModel = existing.into();
            active.jellyfin_id = ActiveValue::Set(Some(jellyfin_id));
            active.update(db).await.map_err(|e| {
                error!("Db error linking existing album: {e:#?}");
                "database error".to_string()
            })?;
            return Ok(Outcome::Linked);
        }
        ExistingMbidDecision::Proceed => {}
    }

    let mb_rg = musicbrainz
        .get_release_group(&mb_release_group_id)
        .await
        .map_err(|e| match e {
            MbRequestError::Reqwest(err) => {
                error!("MB reqwest error: {err:#?}");
                "musicbrainz request failed".to_string()
            }
            MbRequestError::MusicbrainzError(status, body) => {
                error!("MB error {status}: {body}");
                format!("musicbrainz returned {status}")
            }
        })?;

    let title = mb_rg.title.clone();
    let artist_name = mb_rg
        .artist_credit
        .as_ref()
        .and_then(|credits| credits.first())
        .map(|c| c.name.clone())
        .ok_or_else(|| "release group has no artist credit".to_string())?;

    let query = format!("{artist_name} {title}");
    let first_hit = {
        let hits = tidal.find_album(&query).await.map_err(|e| {
            error!("Tidal find_album failed: {e:#?}");
            "tidal search failed".to_string()
        })?;
        hits.into_iter().next().ok_or_else(|| {
            warn!("No Tidal hits for {query:?}");
            "no tidal matches".to_string()
        })?
    };

    let tidal_artists = tidal
        .get_album_artists(&first_hit.id)
        .await
        .map_err(|e| {
            error!("Tidal get_album_artists failed: {e:#?}");
            "tidal album fetch failed".to_string()
        })?;
    let jf_album = jf_album.clone();
    let outcome = db
        .transaction::<_, Outcome, String>(|txn| {
            Box::pin(async move {
                let primary_tidal_artist = tidal_artists
                    .first()
                    .ok_or_else(|| "tidal album has no artists".to_string())?;

                // The first Tidal credit anchors the MB artist link; the remaining
                // credits are upserted without touching their MB identity.
                let existing_artist = artist::Entity::find_by_id(primary_tidal_artist.id.clone())
                    .one(txn)
                    .await
                    .map_err(|e| {
                        error!("Db error looking up artist: {e:#?}");
                        "database error".to_string()
                    })?;

                match decide_artist(existing_artist, primary_tidal_artist, mb_artist_id.as_deref()) {
                    ArtistDecision::LinkMusicBrainz {
                        existing,
                        mb_artist_id,
                    } => {
                        let mut active: artist::ActiveModel = existing.into();
                        active.musicbrainz_artist_id = Set(Some(mb_artist_id));
                        active.update(txn).await.map_err(|e| {
                            error!("Db error updating artist MB artist ID: {e:#?}");
                            "database error".to_string()
                        })?;
                    }
                    ArtistDecision::Insert {
                        id,
                        name,
                        mb_artist_id,
                    } => {
                        artist::ActiveModel {
                            id: ActiveValue::Set(id),
                            name: ActiveValue::Set(name),
                            musicbrainz_artist_id: ActiveValue::Set(mb_artist_id),
                        }
                        .insert(txn)
                        .await
                        .map_err(|e| {
                            error!("Db error inserting artist: {e:#?}");
                            "database error".to_string()
                        })?;
                    }
                    ArtistDecision::Noop => {}
                }

                for tidal_artist in tidal_artists.iter().skip(1) {
                    upsert_artist(txn, tidal_artist).await?;
                }

                // A row with this Tidal ID may already exist, created earlier via the
                // Tidal-by-ID creation route; link it instead of inserting a duplicate.
                let existing_by_tidal_id = album::Entity::find_by_id(first_hit.id.clone())
                    .one(txn)
                    .await
                    .map_err(|e| {
                        error!("Db error looking up album by Tidal ID: {e:#?}");
                        "database error".to_string()
                    })?;

                let release_date = if existing_by_tidal_id.is_none() {
                    let date = first_hit
                        .release_date
                        .as_deref()
                        .ok_or_else(|| "tidal album has no release date".to_string())?;
                    Some(
                        parse_release_date(date)
                            .map_err(|e| format!("invalid tidal release date: {e}"))?,
                    )
                } else {
                    None
                };

                match decide_album_insert(
                    existing_by_tidal_id,
                    &title,
                    &first_hit,
                    &jf_album,
                    &mb_release_group_id,
                    release_date,
                ) {
                    AlbumDecision::LinkExisting {
                        existing,
                        jellyfin_id,
                        musicbrainz_release_group_id,
                    } => {
                        let mut active: album::ActiveModel = existing.into();
                        active.jellyfin_id = Set(jellyfin_id);
                        active.musicbrainz_release_group_id = Set(musicbrainz_release_group_id);
                        active.update(txn).await.map_err(|e| {
                            error!("Db error linking album by Tidal ID: {e:#?}");
                            "database error".to_string()
                        })?;
                        insert_credits(txn, &first_hit.id, &tidal_artists).await?;
                        Ok(Outcome::Linked)
                    }
                    AlbumDecision::Create {
                        id,
                        title,
                        album_type,
                        jellyfin_id,
                        musicbrainz_release_group_id,
                        release_date,
                    } => {
                        album::ActiveModel {
                            id: ActiveValue::Set(id),
                            title: ActiveValue::Set(title),
                            album_type: ActiveValue::Set(album_type),
                            jellyfin_id: ActiveValue::Set(Some(jellyfin_id)),
                            musicbrainz_release_group_id:
                                ActiveValue::Set(Some(musicbrainz_release_group_id)),
                            match_method: ActiveValue::Set(Some("name_search".to_string())),
                            release_year: ActiveValue::Set(release_date.year),
                            release_month: ActiveValue::Set(release_date.month),
                            release_day: ActiveValue::Set(release_date.day),
                        }
                        .insert(txn)
                        .await
                        .map_err(|e| {
                            error!("Db error inserting album: {e:#?}");
                            "database error".to_string()
                        })?;
                        insert_credits(txn, &first_hit.id, &tidal_artists).await?;
                        Ok(Outcome::Created)
                    }
                }
            })
        })
        .await
        .map_err(|e| {
            error!("Db error resolving album transaction: {e:#?}");
            "database transaction failed".to_string()
        })?;

    Ok(outcome)
}

async fn upsert_artist(
    db: &impl ConnectionTrait,
    tidal_artist: &TidalArtist,
) -> Result<(), String> {
    catalog_utils::upsert_artist(db, tidal_artist).await.map_err(|e| {
        error!("Db error upserting artist: {e:#?}");
        "database error".to_string()
    })
}

async fn insert_credits(
    db: &impl ConnectionTrait,
    album_id: &str,
    tidal_artists: &[TidalArtist],
) -> Result<(), String> {
    catalog_utils::insert_credits(db, album_id, tidal_artists)
        .await
        .map_err(|e| {
            error!("Db error inserting album_artist: {e:#?}");
            "database error".to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{album, artist};
    use crate::services::jellyfin::JellyfinAlbum;
    use crate::services::tidal::{ResolvedTidalSearchedAlbum, TidalArtist};
    use std::collections::HashMap;


    fn jf_album(id: &str) -> JellyfinAlbum {
        JellyfinAlbum {
            id: id.to_string(),
            name: "Some Album".to_string(),
            provider_ids: HashMap::new(),
            production_year: None,
        }
    }

    fn tidal_artist(id: &str, name: &str) -> TidalArtist {
        TidalArtist {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn tidal_hit(id: &str, ty: &str) -> ResolvedTidalSearchedAlbum {
        ResolvedTidalSearchedAlbum {
            id: id.to_string(),
            title: "Some Album".to_string(),
            barcode_id: None,
            number_of_volumes: None,
            number_of_items: None,
            duration: "PT0S".to_string(),
            explicit: false,
            release_date: Some("2020-05-17".to_string()),
            popularity: 0.0,
            access_type: None,
            availability: None,
            media_tags: None,
            r#type: ty.to_string(),
        }
    }

    fn album_model(
        id: &str,
        jellyfin_id: Option<&str>,
        musicbrainz_release_group_id: Option<&str>,
    ) -> album::Model {
        album::Model {
            id: id.to_string(),
            title: "Some Album".to_string(),
            album_type: None,
            jellyfin_id: jellyfin_id.map(str::to_string),
            musicbrainz_release_group_id: musicbrainz_release_group_id.map(str::to_string),
            match_method: None,
            release_year: 2020,
            release_month: Some(5),
            release_day: Some(17),
        }
    }

    fn artist_model(id: &str, name: &str, mb_artist_id: Option<&str>) -> artist::Model {
        artist::Model {
            id: id.to_string(),
            name: name.to_string(),
            musicbrainz_artist_id: mb_artist_id.map(str::to_string),
        }
    }

    #[test]
    fn existing_mbid_none_proceeds() {
        assert_eq!(
            decide_existing_mbid(None, &jf_album("jf-1")),
            ExistingMbidDecision::Proceed,
        );
    }

    #[test]
    fn existing_mbid_with_jellyfin_id_skips() {
        let existing = album_model("mbid-row", Some("old-jf"), Some("mbid-1"));
        assert_eq!(
            decide_existing_mbid(Some(existing), &jf_album("jf-1")),
            ExistingMbidDecision::Skip,
        );
    }

    #[test]
    fn existing_mbid_without_jellyfin_id_links() {
        let existing = album_model("mbid-row", None, Some("mbid-1"));
        assert_eq!(
            decide_existing_mbid(Some(existing.clone()), &jf_album("jf-1")),
            ExistingMbidDecision::Link {
                existing,
                jellyfin_id: "jf-1".to_string()
            },
        );
    }

    #[test]
    fn artist_none_inserts_with_mbid() {
        let ta = tidal_artist("ta-1", "The Artist");
        assert_eq!(
            decide_artist(None, &ta, Some("mb-artist-1")),
            ArtistDecision::Insert {
                id: "ta-1".to_string(),
                name: "The Artist".to_string(),
                mb_artist_id: Some("mb-artist-1".to_string()),
            },
        );
    }

    #[test]
    fn artist_none_inserts_without_mbid() {
        let ta = tidal_artist("ta-1", "The Artist");
        assert_eq!(
            decide_artist(None, &ta, None),
            ArtistDecision::Insert {
                id: "ta-1".to_string(),
                name: "The Artist".to_string(),
                mb_artist_id: None,
            },
        );
    }

    #[test]
    fn artist_existing_without_mbid_links_musicbrainz() {
        let existing = artist_model("ta-1", "The Artist", None);
        let ta = tidal_artist("ta-1", "The Artist");
        assert_eq!(
            decide_artist(Some(existing.clone()), &ta, Some("mb-artist-1")),
            ArtistDecision::LinkMusicBrainz {
                existing,
                mb_artist_id: "mb-artist-1".to_string()
            },
        );
    }

    #[test]
    fn artist_existing_with_mbid_noops() {
        let existing = artist_model("ta-1", "The Artist", Some("mb-artist-1"));
        let ta = tidal_artist("ta-1", "The Artist");
        assert_eq!(
            decide_artist(Some(existing), &ta, Some("mb-artist-2")),
            ArtistDecision::Noop,
        );
    }

    #[test]
    fn artist_existing_without_mbid_and_none_provided_noops() {
        let existing = artist_model("ta-1", "The Artist", None);
        let ta = tidal_artist("ta-1", "The Artist");
        assert_eq!(
            decide_artist(Some(existing), &ta, None),
            ArtistDecision::Noop,
        );
    }

    #[test]
    fn album_none_creates() {
        let hit = tidal_hit("tidal-1", "EP");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                None,
                "Some Album",
                &hit,
                &jf,
                "mbid-1",
                Some(ReleaseDate {
                    year: 2020,
                    month: Some(5),
                    day: Some(17),
                }),
            ),
            AlbumDecision::Create {
                id: "tidal-1".to_string(),
                title: "Some Album".to_string(),
                album_type: Some("EP".to_string()),
                jellyfin_id: "jf-1".to_string(),
                musicbrainz_release_group_id: "mbid-1".to_string(),
                release_date: ReleaseDate {
                    year: 2020,
                    month: Some(5),
                    day: Some(17),
                },
            },
        );
    }

    #[test]
    fn album_existing_without_ids_links_both() {
        let existing = album_model("tidal-1", None, None);
        let hit = tidal_hit("tidal-1", "ALBUM");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                Some(existing.clone()),
                "Some Album",
                &hit,
                &jf,
                "mbid-1",
                None,
            ),
            AlbumDecision::LinkExisting {
                existing,
                jellyfin_id: Some("jf-1".to_string()),
                musicbrainz_release_group_id: Some("mbid-1".to_string()),
            },
        );
    }

    #[test]
    fn album_existing_with_jellyfin_id_preserves_it() {
        let existing = album_model("tidal-1", Some("old-jf"), None);
        let hit = tidal_hit("tidal-1", "ALBUM");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                Some(existing.clone()),
                "Some Album",
                &hit,
                &jf,
                "mbid-1",
                None,
            ),
            AlbumDecision::LinkExisting {
                existing,
                jellyfin_id: Some("old-jf".to_string()),
                musicbrainz_release_group_id: Some("mbid-1".to_string()),
            },
        );
    }

    #[test]
    fn album_existing_with_mbid_preserves_it() {
        let existing = album_model("tidal-1", None, Some("old-mbid"));
        let hit = tidal_hit("tidal-1", "ALBUM");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                Some(existing.clone()),
                "Some Album",
                &hit,
                &jf,
                "mbid-1",
                None,
            ),
            AlbumDecision::LinkExisting {
                existing,
                jellyfin_id: Some("jf-1".to_string()),
                musicbrainz_release_group_id: Some("old-mbid".to_string()),
            },
        );
    }

    #[test]
    fn album_existing_with_both_ids_preserves_both() {
        let existing = album_model("tidal-1", Some("old-jf"), Some("old-mbid"));
        let hit = tidal_hit("tidal-1", "ALBUM");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                Some(existing.clone()),
                "Some Album",
                &hit,
                &jf,
                "mbid-1",
                None,
            ),
            AlbumDecision::LinkExisting {
                existing,
                jellyfin_id: Some("old-jf".to_string()),
                musicbrainz_release_group_id: Some("old-mbid".to_string()),
            },
        );
    }
}
