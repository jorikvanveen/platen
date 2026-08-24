use axum::{Json, extract::State};
use reqwest::StatusCode;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter, Set,
};
use tracing::{error, info, warn};

use crate::{
    AppState,
    entity::{album, artist},
    services::musicbrainz::RequestError as MbRequestError,
};

pub mod dto {
    use serde::Serialize;
    use ts_rs::TS;

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct ImportFailure {
        pub name: String,
        pub reason: String,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct ImportSummary {
        pub total_scanned: u32,
        pub created: u32,
        pub linked: u32,
        pub skipped: u32,
        pub failed: u32,
        pub failures: Vec<ImportFailure>,
    }
}

#[axum::debug_handler]
pub async fn import(
    State(AppState {
        musicbrainz,
        jellyfin,
        tidal,
        db,
        ..
    }): State<AppState>,
) -> Result<Json<dto::ImportSummary>, StatusCode> {
    info!("Starting Jellyfin import");

    let jellyfin_albums = jellyfin.list_albums().await.map_err(|e| {
        error!("Jellyfin list_albums failed: {e:#?}");
        StatusCode::BAD_GATEWAY
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
                summary
                    .failures
                    .push(dto::ImportFailure { name: jf_album.name, reason });
            }
        }
    }

    Ok(Json(summary))
}

enum Outcome {
    Linked,
    Created,
    Skipped,
}

/// Decision for the existing-album-by-MBID lookup phase.
#[derive(Debug, PartialEq)]
enum ExistingMbidDecision {
    /// An album row already has a Jellyfin ID; nothing to do.
    Skip,
    /// An album row exists but lacks a Jellyfin ID; link it.
    Link { jellyfin_id: String },
    /// No row by MBID; proceed to the Tidal search path.
    Proceed,
}

/// Decision for the artist upsert phase.
#[derive(Debug, PartialEq)]
enum ArtistDecision {
    /// Existing artist row; link it to MusicBrainz by setting its missing
    /// `musicbrainz_artist_id`.
    LinkMusicBrainz { mb_artist_id: String },
    /// No artist row; insert a new one.
    Insert { id: String, name: String, mb_artist_id: Option<String> },
    /// Existing artist row already complete (or no MB artist ID to link).
    Noop,
}

/// Decision for the album insert/link phase.
#[derive(Debug, PartialEq)]
enum AlbumDecision {
    /// A row with this Tidal ID exists; link Jellyfin/MB release group IDs onto it.
    LinkExisting { jellyfin_id: Option<String>, musicbrainz_release_group_id: Option<String> },
    /// No row; create a new album keyed by the Tidal ID.
    Create {
        id: String,
        artist_id: String,
        title: String,
        album_type: Option<String>,
        jellyfin_id: String,
        musicbrainz_release_group_id: String,
    },
}

/// Decide what to do given an existing album row looked up by MBID.
///
/// Pure: no I/O, no `sea-orm` types in the return.
fn decide_existing_mbid(
    existing_by_mbid: Option<&album::Model>,
    jf_album: &crate::services::jellyfin::JellyfinAlbum,
) -> ExistingMbidDecision {
    match existing_by_mbid {
        Some(existing) if existing.jellyfin_id.is_some() => ExistingMbidDecision::Skip,
        Some(_) => ExistingMbidDecision::Link { jellyfin_id: jf_album.id.clone() },
        None => ExistingMbidDecision::Proceed,
    }
}

/// Decide what to do for the artist row keyed by the Tidal artist ID.
///
/// Pure: takes the existing artist row (if any), the Tidal artist, and the
/// optional MusicBrainz artist ID Jellyfin may have provided.
fn decide_artist(
    existing_artist: Option<&artist::Model>,
    tidal_artist: &crate::services::tidal::TidalArtist,
    mb_artist_id: Option<&str>,
) -> ArtistDecision {
    match existing_artist {
        Some(existing) => {
            if existing.musicbrainz_artist_id.is_none() && mb_artist_id.is_some() {
                ArtistDecision::LinkMusicBrainz { mb_artist_id: mb_artist_id.unwrap().to_string() }
            } else {
                ArtistDecision::Noop
            }
        }
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
/// Pure: the shell is responsible for translating `LinkExisting`/`Create`
/// into `ActiveModel` writes.
#[allow(clippy::too_many_arguments)]
fn decide_album_insert(
    existing_by_tidal_id: Option<&album::Model>,
    title: &str,
    tidal_hit: &crate::services::tidal::ResolvedTidalSearchedAlbum,
    tidal_artist: &crate::services::tidal::TidalArtist,
    jf_album: &crate::services::jellyfin::JellyfinAlbum,
    mb_release_group_id: &str,
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
            AlbumDecision::LinkExisting { jellyfin_id, musicbrainz_release_group_id }
        }
        None => AlbumDecision::Create {
            id: tidal_hit.id.clone(),
            artist_id: tidal_artist.id.clone(),
            title: title.to_string(),
            album_type: tidal_hit.album_type.clone(),
            jellyfin_id: jf_album.id.clone(),
            musicbrainz_release_group_id: mb_release_group_id.to_string(),
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

    match decide_existing_mbid(existing_by_mbid.as_ref(), jf_album) {
        ExistingMbidDecision::Skip => return Ok(Outcome::Skipped),
        ExistingMbidDecision::Link { jellyfin_id } => {
            let mut active: album::ActiveModel = existing_by_mbid.unwrap().into();
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

    match decide_artist(existing_artist.as_ref(), &tidal_artist, mb_artist_id.as_deref()) {
        ArtistDecision::LinkMusicBrainz { mb_artist_id } => {
            let mut active: artist::ActiveModel = existing_artist.unwrap().into();
            active.musicbrainz_artist_id = Set(Some(mb_artist_id));
            active.update(db).await.map_err(|e| {
                error!("Db error updating artist MB artist ID: {e:#?}");
                "database error".to_string()
            })?;
        }
        ArtistDecision::Insert { id, name, mb_artist_id } => {
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

    match decide_album_insert(
        existing_by_tidal_id.as_ref(),
        &title,
        &first_hit,
        &tidal_artist,
        jf_album,
        &mb_release_group_id,
    ) {
        AlbumDecision::LinkExisting { jellyfin_id, musicbrainz_release_group_id } => {
            let mut active: album::ActiveModel = existing_by_tidal_id.unwrap().into();
            active.jellyfin_id = Set(jellyfin_id);
            active.musicbrainz_release_group_id = Set(musicbrainz_release_group_id);
            active.update(db).await.map_err(|e| {
                error!("Db error linking album by Tidal ID: {e:#?}");
                "database error".to_string()
            })?;
            Ok(Outcome::Linked)
        }
        AlbumDecision::Create { id, artist_id, title, album_type, jellyfin_id, musicbrainz_release_group_id } => {
            album::ActiveModel {
                id: ActiveValue::Set(id),
                artist_id: ActiveValue::Set(artist_id),
                title: ActiveValue::Set(title),
                album_type: ActiveValue::Set(album_type),
                jellyfin_id: ActiveValue::Set(Some(jellyfin_id)),
                musicbrainz_release_group_id: ActiveValue::Set(Some(musicbrainz_release_group_id)),
                match_method: ActiveValue::Set(Some("name_search".to_string())),
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

    fn jf_album(id: &str) -> JellyfinAlbum {
        JellyfinAlbum {
            id: id.to_string(),
            name: "Some Album".to_string(),
            provider_ids: HashMap::new(),
            production_year: None,
        }
    }

    fn tidal_artist(id: &str, name: &str) -> TidalArtist {
        TidalArtist { id: id.to_string(), name: name.to_string() }
    }

    fn tidal_hit(id: &str, album_type: Option<&str>) -> ResolvedTidalSearchedAlbum {
        ResolvedTidalSearchedAlbum {
            id: id.to_string(),
            title: "Some Album".to_string(),
            barcode_id: None,
            number_of_volumes: None,
            number_of_items: None,
            duration: "PT0S".to_string(),
            explicit: false,
            release_date: None,
            popularity: 0.0,
            access_type: None,
            availability: None,
            media_tags: None,
            r#type: "ALBUM".to_string(),
            album_type: album_type.map(str::to_string),
        }
    }

    fn album_model(id: &str, jellyfin_id: Option<&str>, musicbrainz_release_group_id: Option<&str>) -> album::Model {
        album::Model {
            id: id.to_string(),
            artist_id: "artist-1".to_string(),
            title: "Some Album".to_string(),
            album_type: None,
            jellyfin_id: jellyfin_id.map(str::to_string),
            musicbrainz_release_group_id: musicbrainz_release_group_id.map(str::to_string),
            match_method: None,
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
            decide_existing_mbid(Some(&existing), &jf_album("jf-1")),
            ExistingMbidDecision::Skip,
        );
    }

    #[test]
    fn existing_mbid_without_jellyfin_id_links() {
        let existing = album_model("mbid-row", None, Some("mbid-1"));
        assert_eq!(
            decide_existing_mbid(Some(&existing), &jf_album("jf-1")),
            ExistingMbidDecision::Link { jellyfin_id: "jf-1".to_string() },
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
            decide_artist(Some(&existing), &ta, Some("mb-artist-1")),
            ArtistDecision::LinkMusicBrainz { mb_artist_id: "mb-artist-1".to_string() },
        );
    }

    #[test]
    fn artist_existing_with_mbid_noops() {
        let existing = artist_model("ta-1", "The Artist", Some("mb-artist-1"));
        let ta = tidal_artist("ta-1", "The Artist");
        assert_eq!(
            decide_artist(Some(&existing), &ta, Some("mb-artist-2")),
            ArtistDecision::Noop,
        );
    }

    #[test]
    fn artist_existing_without_mbid_and_none_provided_noops() {
        let existing = artist_model("ta-1", "The Artist", None);
        let ta = tidal_artist("ta-1", "The Artist");
        assert_eq!(
            decide_artist(Some(&existing), &ta, None),
            ArtistDecision::Noop,
        );
    }

    // --- decide_album_insert ---

    #[test]
    fn album_none_creates() {
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", Some("ALBUM"));
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(None, "Some Album", &hit, &ta, &jf, "mbid-1"),
            AlbumDecision::Create {
                id: "tidal-1".to_string(),
                artist_id: "ta-1".to_string(),
                title: "Some Album".to_string(),
                album_type: Some("ALBUM".to_string()),
                jellyfin_id: "jf-1".to_string(),
                musicbrainz_release_group_id: "mbid-1".to_string(),
            },
        );
    }

    #[test]
    fn album_none_creates_with_null_album_type() {
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", None);
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(None, "Some Album", &hit, &ta, &jf, "mbid-1"),
            AlbumDecision::Create {
                id: "tidal-1".to_string(),
                artist_id: "ta-1".to_string(),
                title: "Some Album".to_string(),
                album_type: None,
                jellyfin_id: "jf-1".to_string(),
                musicbrainz_release_group_id: "mbid-1".to_string(),
            },
        );
    }

    #[test]
    fn album_existing_without_ids_links_both() {
        let existing = album_model("tidal-1", None, None);
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", Some("ALBUM"));
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(Some(&existing), "Some Album", &hit, &ta, &jf, "mbid-1"),
            AlbumDecision::LinkExisting {
                jellyfin_id: Some("jf-1".to_string()),
                musicbrainz_release_group_id: Some("mbid-1".to_string()),
            },
        );
    }

    #[test]
    fn album_existing_with_jellyfin_id_preserves_it() {
        let existing = album_model("tidal-1", Some("old-jf"), None);
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", Some("ALBUM"));
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(Some(&existing), "Some Album", &hit, &ta, &jf, "mbid-1"),
            AlbumDecision::LinkExisting {
                jellyfin_id: Some("old-jf".to_string()),
                musicbrainz_release_group_id: Some("mbid-1".to_string()),
            },
        );
    }

    #[test]
    fn album_existing_with_mbid_preserves_it() {
        let existing = album_model("tidal-1", None, Some("old-mbid"));
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", Some("ALBUM"));
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(Some(&existing), "Some Album", &hit, &ta, &jf, "mbid-1"),
            AlbumDecision::LinkExisting {
                jellyfin_id: Some("jf-1".to_string()),
                musicbrainz_release_group_id: Some("old-mbid".to_string()),
            },
        );
    }

    #[test]
    fn album_existing_with_both_ids_preserves_both() {
        let existing = album_model("tidal-1", Some("old-jf"), Some("old-mbid"));
        let ta = tidal_artist("ta-1", "The Artist");
        let hit = tidal_hit("tidal-1", Some("ALBUM"));
        let jf = jf_album("jf-1");
        assert_eq!(
            decide_album_insert(Some(&existing), "Some Album", &hit, &ta, &jf, "mbid-1"),
            AlbumDecision::LinkExisting {
                jellyfin_id: Some("old-jf".to_string()),
                musicbrainz_release_group_id: Some("old-mbid".to_string()),
            },
        );
    }
}
