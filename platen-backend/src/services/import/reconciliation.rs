use std::collections::{BTreeMap, HashMap, HashSet};

use super::model::{AlbumCandidate, CatalogAlbum, LocationUpdate};
use crate::services::filesystem::filesystem_safe_component;

fn normalized_matching_component(value: &str, fallback: &str) -> String {
    filesystem_safe_component(value, fallback)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(super) fn identity(artist: &str, title: &str) -> (String, String) {
    (
        normalized_matching_component(artist, "Unknown artist"),
        normalized_matching_component(title, "Unknown album"),
    )
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ReconciliationPlan {
    pub(super) locations: Vec<LocationUpdate>,
    pub(super) unchanged_locations: usize,
    pub(super) unknown_candidates: Vec<AlbumCandidate>,
    pub(super) ambiguous_candidates: Vec<AlbumCandidate>,
    pub(super) duplicate_paths_by_album_id: BTreeMap<String, Vec<String>>,
}

pub(super) fn reconcile(
    candidates: &[AlbumCandidate],
    catalog_albums: &[CatalogAlbum],
) -> ReconciliationPlan {
    let mut plan = ReconciliationPlan::default();
    let observed_paths: HashSet<&str> = candidates
        .iter()
        .map(|candidate| candidate.relative_path.as_str())
        .collect();
    let catalog_album_id_by_path: HashMap<&str, &str> = catalog_albums
        .iter()
        .filter_map(|album| {
            album
                .relative_path
                .as_deref()
                .map(|path| (path, album.id.as_str()))
        })
        .collect();
    let mut catalog_albums_by_identity: HashMap<(String, String), Vec<&CatalogAlbum>> =
        HashMap::new();
    for album in catalog_albums {
        if let Some(artist) = &album.primary_artist {
            catalog_albums_by_identity
                .entry(identity(artist, &album.title))
                .or_default()
                .push(album);
        }
    }
    let mut candidates_by_album_id: HashMap<&str, Vec<&AlbumCandidate>> = HashMap::new();
    for candidate in candidates {
        // Stored locations retain their identity even after metadata changes.
        if let Some(album_id) = catalog_album_id_by_path.get(candidate.relative_path.as_str()) {
            candidates_by_album_id
                .entry(album_id)
                .or_default()
                .push(candidate);
            continue;
        }
        let matches: Vec<_> = catalog_albums_by_identity
            .get(&identity(&candidate.primary_artist, &candidate.title))
            .into_iter()
            .flatten()
            .filter(|album| {
                candidate
                    .release_year
                    .is_none_or(|year| year == album.release_year)
            })
            .collect();
        match matches.as_slice() {
            [album] => candidates_by_album_id
                .entry(&album.id)
                .or_default()
                .push(candidate),
            [] => plan.unknown_candidates.push(candidate.clone()),
            _ => plan.ambiguous_candidates.push(candidate.clone()),
        }
    }
    for album in catalog_albums {
        let album_candidates = candidates_by_album_id
            .get(album.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        if album_candidates.len() > 1 {
            let mut paths: Vec<_> = album_candidates
                .iter()
                .map(|candidate| candidate.relative_path.clone())
                .collect();
            paths.sort();
            plan.duplicate_paths_by_album_id
                .insert(album.id.clone(), paths);
        }
        let stored_path = album.relative_path.as_deref();
        let decision = decide_location(
            stored_path,
            stored_path.is_some_and(|path| observed_paths.contains(path)),
            album_candidates,
        );
        let relative_path = match decision {
            LocationDecision::Keep(_) => {
                plan.unchanged_locations += 1;
                continue;
            }
            LocationDecision::NoLocation => continue,
            LocationDecision::Attach(path) | LocationDecision::Change(path) => {
                Some(path.to_owned())
            }
            LocationDecision::Clear => None,
        };
        plan.locations.push(LocationUpdate {
            album_id: album.id.clone(),
            previous_path: album.relative_path.clone(),
            relative_path,
        });
    }
    plan.locations
        .sort_by(|left, right| left.album_id.cmp(&right.album_id));
    plan.unknown_candidates
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    plan.ambiguous_candidates
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    plan
}

#[derive(Debug, PartialEq, Eq)]
enum LocationDecision<'a> {
    Keep(&'a str),
    Attach(&'a str),
    Change(&'a str),
    Clear,
    NoLocation,
}

fn decide_location<'a>(
    stored_path: Option<&'a str>,
    observed: bool,
    candidates: &[&'a AlbumCandidate],
) -> LocationDecision<'a> {
    if let Some(path) = stored_path.filter(|_| observed) {
        return LocationDecision::Keep(path);
    }
    let unique_path = match candidates {
        [candidate] => Some(candidate.relative_path.as_str()),
        _ => None,
    };
    match (stored_path, unique_path) {
        (None, Some(path)) => LocationDecision::Attach(path),
        (Some(_), Some(path)) => LocationDecision::Change(path),
        (Some(_), None) => LocationDecision::Clear,
        (None, None) => LocationDecision::NoLocation,
    }
}

#[cfg(test)]
#[path = "../../tests/services/import/reconciliation.rs"]
mod tests;
