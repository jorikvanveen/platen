use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use sea_orm::{ActiveModelTrait, DatabaseConnection, DbErr, EntityTrait, QueryOrder, Set};

use crate::{
    entity::{album, album_artist, artist},
    services::filesystem::filesystem_safe_component,
};

use super::scan::{AlbumCandidate, DiscoveryReport, ScanSummary};

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

pub(super) async fn reconcile(
    db: &DatabaseConnection,
    report: &mut DiscoveryReport,
    summary: &mut ScanSummary,
) -> Result<Vec<AlbumCandidate>, DbErr> {
    report.reconciled_duplicate_paths.clear();
    report.reconciled_duplicate_album_ids.clear();
    let observed_disk_paths: HashSet<&str> = report
        .candidates
        .iter()
        .map(|disk_candidate| disk_candidate.relative_path.as_str())
        .collect();
    let catalog_albums = album::Entity::find().all(db).await?;
    let catalog_album_id_by_path: HashMap<&str, &str> = catalog_albums
        .iter()
        .filter_map(|catalog_album| {
            catalog_album
                .relative_path
                .as_deref()
                .map(|path| (path, catalog_album.id.as_str()))
        })
        .collect();

    let catalog_credits = album_artist::Entity::find()
        .find_both_related(artist::Entity)
        .order_by_asc(album_artist::Column::Position)
        .all(db)
        .await?;
    let mut catalog_primary_artists = HashMap::new();
    for (catalog_credit, catalog_artist) in &catalog_credits {
        catalog_primary_artists
            .entry(catalog_credit.album_id.as_str())
            .or_insert(catalog_artist);
    }
    let mut catalog_albums_by_artist_and_title: HashMap<(String, String), Vec<&album::Model>> =
        HashMap::new();
    for catalog_album in &catalog_albums {
        if let Some(catalog_artist) = catalog_primary_artists.get(catalog_album.id.as_str()) {
            catalog_albums_by_artist_and_title
                .entry(identity(&catalog_artist.name, &catalog_album.title))
                .or_default()
                .push(catalog_album);
        }
    }
    let mut disk_candidates_by_catalog_album_id: HashMap<&str, Vec<&AlbumCandidate>> =
        HashMap::new();
    let mut unknown_candidates = Vec::new();
    for disk_candidate in &report.candidates {
        // An observed stored location keeps its identity even if its name no longer matches metadata.
        if let Some(catalog_album_id) =
            catalog_album_id_by_path.get(disk_candidate.relative_path.as_str())
        {
            summary.candidates_processed += 1;
            disk_candidates_by_catalog_album_id
                .entry(catalog_album_id)
                .or_default()
                .push(disk_candidate);
            continue;
        }
        let matching_catalog_albums: Vec<_> = catalog_albums_by_artist_and_title
            .get(&identity(
                &disk_candidate.primary_artist,
                &disk_candidate.title,
            ))
            .into_iter()
            .flatten()
            .filter(|catalog_album| {
                disk_candidate
                    .release_year
                    .is_none_or(|year| year == catalog_album.release_year)
            })
            .collect();
        match matching_catalog_albums.as_slice() {
            [catalog_album] => disk_candidates_by_catalog_album_id
                .entry(catalog_album.id.as_str())
                .or_default()
                .push(disk_candidate),
            [] => {
                unknown_candidates.push(disk_candidate.clone());
                continue;
            }
            _ => {
                summary.ambiguous_matches = summary.ambiguous_matches.saturating_add(1);
                summary.skipped_directories = summary.skipped_directories.saturating_add(1);
                let album_ids: Vec<_> = matching_catalog_albums
                    .iter()
                    .map(|album| album.id.as_str())
                    .collect();
                tracing::warn!(reason = "ambiguous_catalog_match", path = %report.absolute_path(&disk_candidate.relative_path).display(), ?album_ids, "Skipped directory during Catalog reconciliation");
            }
        }
        summary.candidates_processed += 1;
    }
    for (catalog_album_id, disk_candidates) in &disk_candidates_by_catalog_album_id {
        if disk_candidates.len() > 1 {
            report
                .reconciled_duplicate_album_ids
                .insert((*catalog_album_id).to_owned());
            for disk_candidate in disk_candidates {
                report
                    .reconciled_duplicate_paths
                    .insert(disk_candidate.relative_path.clone());
                summary.duplicate_locations = summary.duplicate_locations.saturating_add(1);
                summary.skipped_directories = summary.skipped_directories.saturating_add(1);
                tracing::warn!(reason = "duplicate_album_location", path = %report.absolute_path(&disk_candidate.relative_path).display(), album_id = catalog_album_id, "Skipped duplicate Album location");
            }
        }
    }

    for catalog_album in &catalog_albums {
        let stored_path = catalog_album.relative_path.as_deref();
        let disk_candidates = disk_candidates_by_catalog_album_id
            .get(catalog_album.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let decision = decide_location(
            stored_path,
            stored_path.is_some_and(|path| observed_disk_paths.contains(path)),
            disk_candidates,
        );
        apply_location_decision(
            db,
            catalog_album,
            decision,
            summary,
            report.resolved_root.as_deref(),
        )
        .await;
    }

    Ok(unknown_candidates)
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
    stored_path_observed: bool,
    candidates: &[&'a AlbumCandidate],
) -> LocationDecision<'a> {
    if let Some(path) = stored_path.filter(|_| stored_path_observed) {
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

async fn apply_location_decision(
    db: &DatabaseConnection,
    album: &album::Model,
    decision: LocationDecision<'_>,
    summary: &mut ScanSummary,
    root: Option<&Path>,
) {
    let (new_path, reason, count) = match decision {
        LocationDecision::Keep(path) => {
            summary.unchanged_locations = summary.unchanged_locations.saturating_add(1);
            tracing::info!(reason = "location_observed", path = %root.unwrap_or(Path::new("")).join(path).display(), album_id = %album.id, "Kept Album location");
            return;
        }
        LocationDecision::NoLocation => return,
        LocationDecision::Attach(path) => (
            Some(path),
            "location_attached",
            &mut summary.locations_attached,
        ),
        LocationDecision::Change(path) => (
            Some(path),
            "location_changed",
            &mut summary.locations_changed,
        ),
        LocationDecision::Clear => (None, "location_absent", &mut summary.locations_cleared),
    };
    let path = new_path
        .or(album.relative_path.as_deref())
        .unwrap_or_default();
    let result = album::ActiveModel {
        id: Set(album.id.clone()),
        relative_path: Set(new_path.map(str::to_owned)),
        ..Default::default()
    }
    .update(db)
    .await;
    let path = root.unwrap_or(Path::new("")).join(path);
    match result {
        Ok(_) => {
            *count = count.saturating_add(1);
            tracing::info!(reason, path = %path.display(), old_path = ?album.relative_path, album_id = %album.id, "Reconciled Album location");
        }
        Err(error) => {
            summary.failures = summary.failures.saturating_add(1);
            tracing::error!(reason = "persist_album_location", path = %path.display(), album_id = %album.id, %error, "Could not reconcile Album location");
        }
    }
}

#[cfg(test)]
#[path = "../../tests/services/catalog/reconciliation.rs"]
mod tests;
