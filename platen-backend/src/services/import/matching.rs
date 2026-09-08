use std::collections::HashSet;

use super::{model::AlbumCandidate, reconciliation::identity};
use crate::services::{
    catalog::{PreparedAlbum, parse_release_date},
    tidal::TidalCatalog,
};

#[derive(Debug)]
pub(super) enum MatchOutcome {
    Unique(PreparedAlbum),
    Unmatched,
    Ambiguous(Vec<String>),
    Failed,
}

fn matches_identity(candidate: &AlbumCandidate, artist: &str, title: &str) -> bool {
    !artist.trim().is_empty()
        && !title.trim().is_empty()
        && identity(&candidate.primary_artist, &candidate.title) == identity(artist, title)
}

pub(super) async fn match_candidate(
    source: &dyn TidalCatalog,
    candidate: &AlbumCandidate,
) -> MatchOutcome {
    let path = std::path::Path::new(&candidate.relative_path);
    let query = format!("{} {}", candidate.primary_artist, candidate.title);
    let hits = match source.find_album(&query).await {
        Ok(hits) => hits,
        Err(error) => {
            tracing::warn!(reason = "tidal_search_failed", path = %path.display(), %error, "Skipped Tidal candidate");
            return MatchOutcome::Failed;
        }
    };
    let mut seen_ids = HashSet::new();
    let mut verified = Vec::new();
    let mut failed = false;
    for hit in hits {
        let Some(primary_artist) = hit.artists.first() else {
            tracing::warn!(reason = "incomplete_search_credits", path = %path.display(), album_id = %hit.id, "Skipped Tidal candidate");
            failed = true;
            continue;
        };
        if hit.id.trim().is_empty()
            || hit.title.trim().is_empty()
            || primary_artist.name.trim().is_empty()
        {
            tracing::warn!(reason = "incomplete_search_identity", path = %path.display(), album_id = %hit.id, "Skipped Tidal candidate");
            failed = true;
            continue;
        }
        if !matches_identity(candidate, &primary_artist.name, &hit.title) {
            continue;
        }
        let Some(date) = hit
            .release_date
            .as_deref()
            .and_then(|date| parse_release_date(date).ok())
        else {
            tracing::warn!(reason = "invalid_search_release_date", path = %path.display(), album_id = %hit.id, "Skipped Tidal candidate");
            failed = true;
            continue;
        };
        if candidate.release_year.is_some_and(|year| year != date.year)
            || !seen_ids.insert(hit.id.clone())
        {
            continue;
        }
        let album_id = hit.id.clone();
        match PreparedAlbum::try_from(hit) {
            Ok(prepared) => verified.push(prepared),
            Err(error) => {
                tracing::warn!(reason = "invalid_search_metadata", path = %path.display(), %album_id, %error, "Skipped Tidal candidate");
                failed = true;
            }
        }
    }
    // An unverifiable alternative must not make another result look unique.
    if failed {
        MatchOutcome::Failed
    } else if verified.len() > 1 {
        MatchOutcome::Ambiguous(
            verified
                .iter()
                .map(|prepared| prepared.album().id.clone())
                .collect(),
        )
    } else if let Some(prepared) = verified.pop() {
        MatchOutcome::Unique(prepared)
    } else {
        MatchOutcome::Unmatched
    }
}
