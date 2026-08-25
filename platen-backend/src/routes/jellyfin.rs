use std::sync::Arc;

use axum::{Json, extract::State};
use reqwest::StatusCode;
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter, Set};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::{
    AppState,
    entity::{album, artist},
    routes::album::{ReleaseDate, parse_release_date},
    services::musicbrainz::RequestError as MbRequestError,
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
/// body for a rejected concurrent import (R4); `BadGateway` preserves the
/// existing empty-body `502` for an unreachable Jellyfin server.
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

/// In-process import state shared across handlers.
///
/// `running` is true only while a [`RunningGuard`] exists. The guard does not
/// hold the mutex for the import's duration; it locks only briefly to flip
/// `running` on `acquire` and off `Drop`. The long Tidal/MusicBrainz awaits run
/// with the lock free, so [`status`] can `lock().await` and observe `running`
/// mid-import (R5, R8). `Drop` clears the flag on every exit path: success
/// (after [`RunningGuard::finish`] already cleared it), the `BAD_GATEWAY` early
/// return, and panic unwind (best-effort `try_lock`).
#[derive(Debug, Default)]
pub struct ImportState {
    pub running: bool,
    pub last_summary: Option<dto::ImportSummary>,
}

/// RAII guard that keeps `import_state.running = true` while alive and clears
/// it on `Drop`. Build it with [`RunningGuard::acquire`], which rejects a
/// concurrent import by returning the [`dto::ImportStatus`] to send as `409`.
///
/// The guard borrows the shared `Arc<Mutex<ImportState>>` but does not hold the
/// mutex guard across the import body. `acquire` locks only long enough to flip
/// `running` to `true`; [`RunningGuard::finish`] locks only long enough to write
/// `last_summary` and flip `running` back to `false`. This keeps the mutex free
/// during the minutes-long external awaits so [`status`] can read the state
/// without blocking for the whole import (R5, R8).
#[derive(Debug)]
pub struct RunningGuard<'a> {
    state: &'a Arc<Mutex<ImportState>>,
    spent: bool,
}

impl<'a> RunningGuard<'a> {
    /// Try to start an import. Returns `Err(ImportStatus)` with
    /// `state = Running` and the last completed run's `last_summary` when an
    /// import is already in flight, so the caller can return it as `409`
    /// Conflict (R4).
    ///
    /// Uses `lock().await` rather than `try_lock`: a running import only holds
    /// the mutex during its own `acquire`/`finish`, so a second request waits at
    /// most for that brief window, not for the whole import.
    pub async fn acquire(
        import_state: &'a Arc<Mutex<ImportState>>,
    ) -> Result<Self, dto::ImportStatus> {
        let mut guard = import_state.lock().await;
        if guard.running {
            return Err(dto::ImportStatus {
                state: dto::ImportStateKind::Running,
                last_summary: guard.last_summary.clone(),
            });
        }
        guard.running = true;
        drop(guard);
        Ok(Self {
            state: import_state,
            spent: false,
        })
    }

    /// Record the completed run's summary and clear `running`. Called on the
    /// success path; after this `Drop` is a no-op (R7).
    pub async fn finish(&mut self, summary: dto::ImportSummary) {
        let mut guard = self.state.lock().await;
        guard.last_summary = Some(summary);
        guard.running = false;
        self.spent = true;
    }
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        if self.spent {
            return;
        }
        // Fallback for early returns (`BAD_GATEWAY`) and panic unwind: clear the
        // flag without holding up the dropping task. `try_lock` is safe here
        // because a running import does not hold the mutex between `acquire` and
        // `finish`; the only contender is a brief [`status`] read, so this
        // virtually always succeeds. If it ever does not, a process restart
        // clears the flag (see ADR 0002).
        if let Ok(mut guard) = self.state.try_lock() {
            guard.running = false;
        }
    }
}

#[axum::debug_handler]
pub async fn import(
    State(AppState {
        import_state,
        musicbrainz,
        jellyfin,
        tidal,
        db,
        ..
    }): State<AppState>,
) -> Result<Json<dto::ImportSummary>, ImportError> {
    info!("Starting Jellyfin import");

    let mut guard = match RunningGuard::acquire(&import_state).await {
        Ok(g) => g,
        Err(status) => {
            info!("Rejecting concurrent Jellyfin import");
            return Err(ImportError::Conflict(status));
        }
    };

    let jellyfin_albums = jellyfin.list_albums().await.map_err(|e| {
        error!("Jellyfin list_albums failed: {e:#?}");
        ImportError::BadGateway
    })?;

    let mut summary = dto::ImportSummary {
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
                summary.failures.push(dto::ImportFailure {
                    name: jf_album.name,
                    reason,
                });
            }
        }
    }

    guard.finish(summary.clone()).await;
    Ok(Json(summary))
}

/// `GET /jellyfin/import/status`: always `200` with the current
/// [`dto::ImportStatus`] (R5).
///
/// Uses `lock().await`, not `try_lock` (R8). [`import`] does not hold the mutex
/// across its external awaits, so this only blocks for the brief windows where
/// `acquire` or [`RunningGuard::finish`] are flipping the flag. A poll therefore
/// observes `running` while an import is in flight and reads `last_summary` from
/// the last completed run, even mid-import.
#[axum::debug_handler]
pub async fn status(
    State(AppState { import_state, .. }): State<AppState>,
) -> Json<dto::ImportStatus> {
    let state = import_state.lock().await;
    Json(dto::ImportStatus {
        state: if state.running {
            dto::ImportStateKind::Running
        } else {
            dto::ImportStateKind::Idle
        },
        last_summary: state.last_summary.clone(),
    })
}

enum Outcome {
    Linked,
    Created,
    Skipped,
}

/// Decision for the existing-album-by-MBID lookup phase.
///
/// Transient: returned by `decide_existing_mbid` and consumed by the caller's
/// match in the next statement, so the 216-byte `Link` variant is fine despite
/// the size gap with `Skip`/`Proceed`.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
enum ExistingMbidDecision {
    /// An album row already has a Jellyfin ID; nothing to do.
    Skip,
    /// An album row exists but lacks a Jellyfin ID; link it.
    Link {
        existing: album::Model,
        jellyfin_id: String,
    },
    /// No row by MBID; proceed to the Tidal search path.
    Proceed,
}

/// Decision for the artist upsert phase.
#[derive(Debug, PartialEq)]
enum ArtistDecision {
    /// Existing artist row; link it to MusicBrainz by setting its missing
    /// `musicbrainz_artist_id`.
    LinkMusicBrainz {
        existing: artist::Model,
        mb_artist_id: String,
    },
    /// No artist row; insert a new one.
    Insert {
        id: String,
        name: String,
        mb_artist_id: Option<String>,
    },
    /// Existing artist row already complete (or no MB artist ID to link).
    Noop,
}

/// Decision for the album insert/link phase.
#[derive(Debug, PartialEq)]
enum AlbumDecision {
    /// A row with this Tidal ID exists; link Jellyfin/MB release group IDs onto it.
    LinkExisting {
        existing: album::Model,
        jellyfin_id: Option<String>,
        musicbrainz_release_group_id: Option<String>,
    },
    /// No row; create a new album keyed by the Tidal ID.
    Create {
        id: String,
        artist_id: String,
        title: String,
        album_type: Option<String>,
        jellyfin_id: String,
        musicbrainz_release_group_id: String,
        release_date: ReleaseDate,
    },
}

/// Decide what to do given an existing album row looked up by MBID.
///
/// Pure: takes only plain data (the row and the Jellyfin album) and returns
/// plain data. No I/O, no handles to stateful services.
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

/// Decide what to do for the artist row keyed by the Tidal artist ID.
///
/// Pure: takes only plain data and returns plain data. No I/O, no handles to
/// stateful services.
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

/// Decide whether to link an existing album row (found by Tidal ID) or create
/// a new one.
///
/// Pure: takes only plain data and returns plain data. No I/O, no handles to
/// stateful services. The shell is responsible for translating
/// `LinkExisting`/`Create` into `ActiveModel` writes.
#[allow(clippy::too_many_arguments)]
fn decide_album_insert(
    existing_by_tidal_id: Option<album::Model>,
    title: &str,
    tidal_hit: &crate::services::tidal::ResolvedTidalSearchedAlbum,
    tidal_artist: &crate::services::tidal::TidalArtist,
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
            artist_id: tidal_artist.id.clone(),
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
///
/// Thin I/O shell: fetches rows and service data, delegates each branching
/// decision to a pure function, then translates the decision into DB writes.
async fn resolve_album(
    db: &sea_orm::DatabaseConnection,
    musicbrainz: &crate::services::musicbrainz::Musicbrainz,
    tidal: &std::sync::Arc<tokio::sync::Mutex<crate::services::tidal::Tidal>>,
    jf_album: &crate::services::jellyfin::JellyfinAlbum,
    mb_release_group_id: String,
    mb_artist_id: Option<String>,
) -> Result<Outcome, String> {
    // 1. Look up an existing album by its MusicBrainz release group ID and decide.
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

    // 2. No row by MBID. Fetch the MB release group for title + artist.
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

    // 3. Search Tidal by "{artist} {title}".
    let query = format!("{artist_name} {title}");
    let first_hit = {
        let mut tidal = tidal.lock().await;
        let hits = tidal.find_album(&query).await.map_err(|e| {
            error!("Tidal find_album failed: {e:#?}");
            "tidal search failed".to_string()
        })?;
        hits.into_iter().next().ok_or_else(|| {
            warn!("No Tidal hits for {query:?}");
            "no tidal matches".to_string()
        })?
    };

    // 4. Follow up with GET /albums/{id}?include=artists for the Tidal artist.
    let tidal_artists = {
        let mut tidal = tidal.lock().await;
        tidal.get_album_artists(&first_hit.id).await.map_err(|e| {
            error!("Tidal get_album_artists failed: {e:#?}");
            "tidal album fetch failed".to_string()
        })?
    };
    let tidal_artist = tidal_artists
        .into_iter()
        .next()
        .ok_or_else(|| "tidal album has no artists".to_string())?;

    // 5. Upsert the artist row keyed by Tidal artist ID.
    let existing_artist = artist::Entity::find_by_id(tidal_artist.id.clone())
        .one(db)
        .await
        .map_err(|e| {
            error!("Db error looking up artist: {e:#?}");
            "database error".to_string()
        })?;

    match decide_artist(existing_artist, &tidal_artist, mb_artist_id.as_deref()) {
        ArtistDecision::LinkMusicBrainz {
            existing,
            mb_artist_id,
        } => {
            let mut active: artist::ActiveModel = existing.into();
            active.musicbrainz_artist_id = Set(Some(mb_artist_id));
            active.update(db).await.map_err(|e| {
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
            .insert(db)
            .await
            .map_err(|e| {
                error!("Db error inserting artist: {e:#?}");
                "database error".to_string()
            })?;
        }
        ArtistDecision::Noop => {}
    }

    // 6. Insert the album row, or link it if a row with this Tidal ID already
    //    exists (e.g. created earlier via the Tidal-by-ID creation route).
    let existing_by_tidal_id = album::Entity::find_by_id(first_hit.id.clone())
        .one(db)
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
        Some(parse_release_date(date).map_err(|e| format!("invalid tidal release date: {e}"))?)
    } else {
        None
    };

    match decide_album_insert(
        existing_by_tidal_id,
        &title,
        &first_hit,
        &tidal_artist,
        jf_album,
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
            active.update(db).await.map_err(|e| {
                error!("Db error linking album by Tidal ID: {e:#?}");
                "database error".to_string()
            })?;
            Ok(Outcome::Linked)
        }
        AlbumDecision::Create {
            id,
            artist_id,
            title,
            album_type,
            jellyfin_id,
            musicbrainz_release_group_id,
            release_date,
        } => {
            album::ActiveModel {
                id: ActiveValue::Set(id),
                artist_id: ActiveValue::Set(artist_id),
                title: ActiveValue::Set(title),
                album_type: ActiveValue::Set(album_type),
                jellyfin_id: ActiveValue::Set(Some(jellyfin_id)),
                musicbrainz_release_group_id: ActiveValue::Set(Some(musicbrainz_release_group_id)),
                match_method: ActiveValue::Set(Some("name_search".to_string())),
                release_year: ActiveValue::Set(release_date.year),
                release_month: ActiveValue::Set(release_date.month),
                release_day: ActiveValue::Set(release_date.day),
            }
            .insert(db)
            .await
            .map_err(|e| {
                error!("Db error inserting album: {e:#?}");
                "database error".to_string()
            })?;
            Ok(Outcome::Created)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{album, artist};
    use crate::services::jellyfin::JellyfinAlbum;
    use crate::services::tidal::{ResolvedTidalSearchedAlbum, TidalArtist};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // --- RunningGuard ---

    // The guard does not hold the mutex between `acquire` and `Drop`; it only
    // locks briefly to flip `running` on `acquire` and off `Drop`. So reading
    // `running` while the guard is live is a separate `lock().await`, which is
    // safe under the single-threaded `#[tokio::test]` runtime because no lock is
    // held across the await.

    #[tokio::test]
    async fn running_guard_marks_running_while_held() {
        let state = Arc::new(Mutex::new(ImportState::default()));
        let _guard = RunningGuard::acquire(&state)
            .await
            .expect("acquire when idle");
        assert!(
            state.lock().await.running,
            "running must be true while the guard is held"
        );
    }

    #[tokio::test]
    async fn running_guard_clears_running_on_drop() {
        let state = Arc::new(Mutex::new(ImportState::default()));
        {
            let _guard = RunningGuard::acquire(&state)
                .await
                .expect("acquire when idle");
            assert!(
                state.lock().await.running,
                "running must be true while the guard is held"
            );
        }
        assert!(
            !state.lock().await.running,
            "running must be false after the guard drops"
        );
    }

    #[tokio::test]
    async fn running_guard_rejects_second_acquire_while_held() {
        let state = Arc::new(Mutex::new(ImportState::default()));
        let _first = RunningGuard::acquire(&state)
            .await
            .expect("first acquire when idle");
        let status = RunningGuard::acquire(&state)
            .await
            .expect_err("a second acquire while running must be rejected");
        assert_eq!(status.state, dto::ImportStateKind::Running);
        assert!(status.last_summary.is_none());
    }

    #[tokio::test]
    async fn running_guard_allows_reacquire_after_drop() {
        let state = Arc::new(Mutex::new(ImportState::default()));
        {
            let _guard = RunningGuard::acquire(&state)
                .await
                .expect("acquire when idle");
        }
        RunningGuard::acquire(&state)
            .await
            .expect("a fresh acquire must succeed once the guard has dropped");
    }

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
            artist_id: "artist-1".to_string(),
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

    // --- decide_existing_mbid ---

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

    // --- decide_artist ---

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

    // --- decide_album_insert ---

    #[test]
    fn album_none_creates() {
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", "EP");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                None,
                "Some Album",
                &hit,
                &ta,
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
                artist_id: "ta-1".to_string(),
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
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", "ALBUM");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                Some(existing.clone()),
                "Some Album",
                &hit,
                &ta,
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
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", "ALBUM");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                Some(existing.clone()),
                "Some Album",
                &hit,
                &ta,
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
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", "ALBUM");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                Some(existing.clone()),
                "Some Album",
                &hit,
                &ta,
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
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", "ALBUM");
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(
                Some(existing.clone()),
                "Some Album",
                &hit,
                &ta,
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
