use std::collections::{BTreeMap, HashSet};

use futures_util::{StreamExt, stream};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};

use crate::{entity::album, routes::album::parse_release_date, services::tidal::TidalCatalog};

use super::{
    reconciliation::identity,
    scan::{ActiveScan, AlbumCandidate, DiscoveryReport, ScanSummary},
    utils::{PreparedAlbum, persist_album},
};

#[derive(Debug)]
enum MatchOutcome {
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

async fn match_candidate(
    source: &dyn TidalCatalog,
    report: &DiscoveryReport,
    candidate: &AlbumCandidate,
) -> MatchOutcome {
    let path = report.absolute_path(&candidate.relative_path);
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

pub(super) async fn match_and_import(
    db: &DatabaseConnection,
    source: &dyn TidalCatalog,
    report: &DiscoveryReport,
    candidates: Vec<AlbumCandidate>,
    summary: &mut ScanSummary,
    scan: &ActiveScan,
) {
    let mut matches_by_album_id: BTreeMap<String, Vec<(AlbumCandidate, PreparedAlbum)>> =
        BTreeMap::new();
    let mut pending = stream::iter(candidates.into_iter().map(|candidate| async move {
        let outcome = match_candidate(source, report, &candidate).await;
        (candidate, outcome)
    }))
    .buffer_unordered(2);
    while let Some((candidate, outcome)) = pending.next().await {
        summary.candidates_processed += 1;
        let path = report.absolute_path(&candidate.relative_path);
        match outcome {
            MatchOutcome::Unique(prepared) => {
                matches_by_album_id
                    .entry(prepared.album().id.clone())
                    .or_default()
                    .push((candidate, prepared));
            }
            MatchOutcome::Unmatched => {
                summary.unmatched_candidates += 1;
                summary.skipped_directories += 1;
                tracing::info!(reason = "no_tidal_match", path = %path.display(), "Skipped Tidal candidate");
            }
            MatchOutcome::Ambiguous(album_ids) => {
                summary.ambiguous_matches += 1;
                summary.skipped_directories += 1;
                tracing::warn!(reason = "ambiguous_tidal_match", path = %path.display(), ?album_ids, "Skipped Tidal candidate");
            }
            MatchOutcome::Failed => {
                summary.failures += 1;
                summary.skipped_directories += 1;
            }
        }
        scan.publish_summary(summary).await;
    }

    for (album_id, matches) in matches_by_album_id {
        let existing = match album::Entity::find_by_id(&album_id).one(db).await {
            Ok(existing) => existing,
            Err(error) => {
                for (candidate, _) in matches {
                    record_persistence_failure(report, &candidate, &album_id, &error, summary);
                }
                scan.publish_summary(summary).await;
                continue;
            }
        };
        let stored_other_path = existing
            .as_ref()
            .and_then(|album| album.relative_path.as_deref())
            .filter(|path| {
                !matches
                    .iter()
                    .any(|(candidate, _)| candidate.relative_path == *path)
            });
        if matches.len() > 1
            || stored_other_path.is_some()
            || report.reconciled_duplicate_album_ids.contains(&album_id)
        {
            if let Some(path) = stored_other_path {
                if !report.reconciled_duplicate_paths.contains(path)
                    && report
                        .candidates
                        .iter()
                        .any(|candidate| candidate.relative_path == path)
                {
                    summary.duplicate_locations += 1;
                    summary.skipped_directories += 1;
                }
                tracing::warn!(reason = "duplicate_album_location", path = %report.absolute_path(path).display(), %album_id, "Kept existing duplicate Album location");
            }
            for (candidate, _) in matches {
                summary.duplicate_locations += 1;
                summary.skipped_directories += 1;
                tracing::warn!(reason = "duplicate_album_location", path = %report.absolute_path(&candidate.relative_path).display(), %album_id, "Skipped duplicate Album location");
            }
        } else {
            for (candidate, prepared) in matches {
                if let Err(error) = import_match(db, report, &candidate, prepared, summary).await {
                    record_persistence_failure(report, &candidate, &album_id, &error, summary);
                }
            }
        }
        scan.publish_summary(summary).await;
    }
}

fn record_persistence_failure(
    report: &DiscoveryReport,
    candidate: &AlbumCandidate,
    album_id: &str,
    error: &dyn std::fmt::Display,
    summary: &mut ScanSummary,
) {
    summary.failures += 1;
    summary.skipped_directories += 1;
    tracing::error!(reason = "persist_tidal_match_failed", path = %report.absolute_path(&candidate.relative_path).display(), album_id, %error, "Could not persist Tidal match");
}

async fn import_match(
    db: &DatabaseConnection,
    report: &DiscoveryReport,
    candidate: &AlbumCandidate,
    prepared: PreparedAlbum,
    summary: &mut ScanSummary,
) -> Result<(), DbErr> {
    let album_id = prepared.album().id.clone();
    let path = report.absolute_path(&candidate.relative_path);
    if let Some(occupant) = album::Entity::find()
        .filter(album::Column::RelativePath.eq(&candidate.relative_path))
        .one(db)
        .await?
        .filter(|album| album.id != album_id)
    {
        summary.skipped_directories += 1;
        summary.duplicate_locations += 1;
        tracing::warn!(reason = "album_location_conflict", path = %path.display(), %album_id, conflicting_album_id = %occupant.id, "Skipped occupied Album location");
        return Ok(());
    }
    let outcome = persist_album(db, prepared, Some(candidate.relative_path.clone()))
        .await
        .map_err(|error| DbErr::Custom(error.to_string()))?;
    if outcome.imported {
        summary.albums_imported += 1;
        tracing::info!(reason = "album_imported", path = %path.display(), %album_id, "Imported Tidal Album");
        return Ok(());
    }
    if outcome.model.relative_path.as_deref() == Some(&candidate.relative_path) {
        summary.unchanged_locations += 1;
        return Ok(());
    }
    if outcome.model.relative_path.is_some() {
        summary.duplicate_locations += 1;
        summary.skipped_directories += 1;
        tracing::warn!(reason = "concurrent_album_location", path = %path.display(), %album_id, stored_path = ?outcome.model.relative_path, "Skipped duplicate Album location");
        return Ok(());
    }

    // A user may manually add the same album while the scan is matching it, inserting it without a location.
    // Attach the discovered directory only if the location is still unset to avoid overwriting a concurrent update.
    let updated = album::Entity::update_many()
        .col_expr(
            album::Column::RelativePath,
            sea_orm::sea_query::Expr::value(candidate.relative_path.clone()),
        )
        .filter(album::Column::Id.eq(&album_id))
        .filter(album::Column::RelativePath.is_null())
        .exec(db)
        .await?;
    if updated.rows_affected == 1 {
        summary.locations_attached += 1;
        tracing::info!(reason = "location_attached", path = %path.display(), %album_id, "Attached Tidal Album location");
        return Ok(());
    }

    summary.duplicate_locations += 1;
    summary.skipped_directories += 1;
    tracing::warn!(reason = "concurrent_album_location", path = %path.display(), %album_id, "Skipped changed Album location");
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/services/catalog/matching.rs"]
mod tests;
