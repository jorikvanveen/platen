use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use futures_util::{StreamExt, stream};
use sea_orm::{DatabaseConnection, DbErr, TransactionError};

use super::{
    discovery::{DiscoveryProgress, DiscoveryReport, discover_album_candidates},
    matching::{MatchOutcome, match_candidate},
    model::{AlbumCandidate, ImportOutcome, LocationOutcome, ScanPhase, ScanSnapshot, ScanSummary},
    reconciliation::{ReconciliationPlan, reconcile},
    repository::ImportRepository,
};
use crate::services::{
    catalog::PreparedAlbum, music_directory::MusicDirectory, tidal::TidalCatalog,
};

const MATCH_CONCURRENCY: usize = 2;

impl From<&DiscoveryReport> for ScanSummary {
    fn from(report: &DiscoveryReport) -> Self {
        Self {
            album_directories_found: report.candidates.len(),
            candidates_total: report.candidates.len(),
            filesystem_errors: report.diagnostics.len(),
            skipped_directories: report.skipped_directories,
            ..Default::default()
        }
    }
}

#[derive(Clone)]
pub(super) struct ImportWorkflow {
    music_directory: MusicDirectory,
    repository: ImportRepository,
    tidal: Arc<dyn TidalCatalog>,
}

impl ImportWorkflow {
    pub(super) fn new(
        music_directory: MusicDirectory,
        db: DatabaseConnection,
        tidal: Arc<dyn TidalCatalog>,
    ) -> Self {
        Self {
            music_directory,
            repository: ImportRepository::new(db),
            tidal,
        }
    }

    pub(super) async fn run(&self, publish: impl Fn(ScanSnapshot) + Send + Sync) -> ScanSnapshot {
        let mut progress = Progress::new(publish);
        let (report, plan) = {
            let _directory_guard = self.music_directory.lock().await;
            let report = discover_album_candidates(self.music_directory.path(), |discovered| {
                progress.discovery(discovered)
            })
            .await;
            progress.snapshot.summary = ScanSummary::from(&report);
            progress.publish();
            let plan = reconcile_catalog(&self.repository, &report, &mut progress).await;
            (report, plan)
        };
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                tracing::error!(%error, "Could not reconcile Catalog locations");
                progress
                    .snapshot
                    .fail("Could not reconcile Catalog locations.");
                return progress.snapshot;
            }
        };
        if report.root_failed {
            progress
                .snapshot
                .fail("Could not scan the Music directory.");
            return progress.snapshot;
        }
        match_and_import(
            &self.repository,
            self.tidal.as_ref(),
            &report,
            plan,
            &mut progress,
        )
        .await;
        progress.snapshot.phase = ScanPhase::Completed;
        progress.snapshot
    }
}

struct Progress<F> {
    snapshot: ScanSnapshot,
    duplicate_paths: HashSet<String>,
    publish_snapshot: F,
}

impl<F: Fn(ScanSnapshot)> Progress<F> {
    fn new(publish_snapshot: F) -> Self {
        Self {
            snapshot: ScanSnapshot::default(),
            duplicate_paths: HashSet::new(),
            publish_snapshot,
        }
    }

    fn publish(&self) {
        (self.publish_snapshot)(self.snapshot.clone());
    }

    fn discovery(&mut self, discovered: DiscoveryProgress) {
        self.snapshot.summary.album_directories_found = discovered.album_directories_found;
        self.snapshot.summary.skipped_directories = discovered.skipped_directories;
        self.snapshot.summary.filesystem_errors = discovered.filesystem_errors;
        self.publish();
    }

    fn location(&mut self, outcome: LocationOutcome) {
        match outcome {
            LocationOutcome::Attached => self.snapshot.summary.locations_attached += 1,
            LocationOutcome::Changed => self.snapshot.summary.locations_changed += 1,
            LocationOutcome::Cleared => self.snapshot.summary.locations_cleared += 1,
            LocationOutcome::Unchanged => self.snapshot.summary.unchanged_locations += 1,
        }
    }

    fn duplicate(&mut self, report: &DiscoveryReport, path: &str, album_id: &str) {
        if self.duplicate_paths.insert(path.to_owned()) {
            self.snapshot.summary.duplicate_locations += 1;
            self.snapshot.summary.skipped_directories += 1;
            tracing::warn!(reason = "duplicate_album_location", path = %report.absolute_path(path).display(), album_id, "Skipped duplicate Album location");
        }
    }

    fn persistence_failure(
        &mut self,
        report: &DiscoveryReport,
        path: &str,
        album_id: &str,
        error: &dyn std::fmt::Display,
    ) {
        self.snapshot.summary.failures += 1;
        self.snapshot.summary.skipped_directories += 1;
        tracing::error!(reason = "persist_tidal_match_failed", path = %report.absolute_path(path).display(), album_id, %error, "Could not persist Tidal match");
    }
}

async fn reconcile_catalog<F: Fn(ScanSnapshot)>(
    repository: &ImportRepository,
    report: &DiscoveryReport,
    progress: &mut Progress<F>,
) -> Result<ReconciliationPlan, TransactionError<DbErr>> {
    let mut plan = reconcile(&report.candidates, &repository.albums().await?);
    progress.snapshot.summary.candidates_processed =
        report.candidates.len() - plan.unknown_candidates.len();
    progress.snapshot.summary.unchanged_locations += plan.unchanged_locations;
    for candidate in &plan.ambiguous_candidates {
        progress.snapshot.summary.ambiguous_matches += 1;
        progress.snapshot.summary.skipped_directories += 1;
        tracing::warn!(reason = "ambiguous_catalog_match", path = %report.absolute_path(&candidate.relative_path).display(), "Skipped ambiguous Catalog match");
    }
    for (album_id, paths) in &plan.duplicate_paths_by_album_id {
        for path in paths {
            progress.duplicate(report, path, album_id);
        }
    }
    for update in std::mem::take(&mut plan.locations) {
        let album_id = update.album_id.clone();
        let path = update
            .relative_path
            .as_ref()
            .or(update.previous_path.as_ref())
            .cloned()
            .unwrap_or_default();
        match repository.apply_location(update).await {
            Ok(outcome) => {
                tracing::info!(path = %report.absolute_path(&path).display(), %album_id, ?outcome, "Reconciled Album location");
                progress.location(outcome);
            }
            Err(error) => {
                progress.snapshot.summary.failures += 1;
                tracing::error!(reason = "persist_album_location", path = %report.absolute_path(&path).display(), %album_id, %error, "Could not reconcile Album location");
            }
        }
        progress.publish();
    }
    progress.publish();
    Ok(plan)
}

async fn match_and_import<F: Fn(ScanSnapshot)>(
    repository: &ImportRepository,
    source: &dyn TidalCatalog,
    report: &DiscoveryReport,
    plan: ReconciliationPlan,
    progress: &mut Progress<F>,
) {
    progress.snapshot.phase = ScanPhase::Matching;
    progress.publish();
    let mut matches_by_album_id: BTreeMap<String, Vec<(AlbumCandidate, PreparedAlbum)>> =
        BTreeMap::new();
    let mut pending = stream::iter(plan.unknown_candidates.into_iter().map(
        |candidate| async move {
            let outcome = match_candidate(source, &candidate).await;
            (candidate, outcome)
        },
    ))
    .buffer_unordered(MATCH_CONCURRENCY);
    while let Some((candidate, outcome)) = pending.next().await {
        progress.snapshot.summary.candidates_processed += 1;
        match outcome {
            MatchOutcome::Unique(prepared) => {
                matches_by_album_id
                    .entry(prepared.album().id.clone())
                    .or_default()
                    .push((candidate, prepared));
            }
            MatchOutcome::Unmatched => {
                progress.snapshot.summary.unmatched_candidates += 1;
                progress.snapshot.summary.skipped_directories += 1;
                tracing::info!(reason = "no_tidal_match", path = %report.absolute_path(&candidate.relative_path).display(), "Skipped Tidal candidate");
            }
            MatchOutcome::Ambiguous(album_ids) => {
                progress.snapshot.summary.ambiguous_matches += 1;
                progress.snapshot.summary.skipped_directories += 1;
                tracing::warn!(reason = "ambiguous_tidal_match", path = %report.absolute_path(&candidate.relative_path).display(), ?album_ids, "Skipped Tidal candidate");
            }
            MatchOutcome::Failed => {
                progress.snapshot.summary.failures += 1;
                progress.snapshot.summary.skipped_directories += 1;
            }
        }
        progress.publish();
    }

    // No import may win merely because its Tidal response arrived before a duplicate's.
    let observed_paths: HashSet<&str> = report
        .candidates
        .iter()
        .map(|candidate| candidate.relative_path.as_str())
        .collect();
    for (album_id, matches) in matches_by_album_id {
        if matches.len() > 1 || plan.duplicate_paths_by_album_id.contains_key(&album_id) {
            match repository.location(&album_id).await {
                Ok(Some(path)) if observed_paths.contains(path.as_str()) => {
                    progress.duplicate(report, &path, &album_id)
                }
                Ok(_) => {}
                Err(error) => {
                    for (candidate, _) in &matches {
                        progress.persistence_failure(
                            report,
                            &candidate.relative_path,
                            &album_id,
                            &error,
                        );
                    }
                    progress.publish();
                    continue;
                }
            }
            for (candidate, _) in matches {
                progress.duplicate(report, &candidate.relative_path, &album_id);
            }
        } else {
            for (candidate, prepared) in matches {
                match repository
                    .import(prepared, candidate.relative_path.clone())
                    .await
                {
                    Ok(ImportOutcome::Imported) => {
                        progress.snapshot.summary.albums_imported += 1;
                        tracing::info!(reason = "album_imported", path = %report.absolute_path(&candidate.relative_path).display(), %album_id, "Imported Tidal Album");
                    }
                    Ok(ImportOutcome::Location(outcome)) => progress.location(outcome),
                    Ok(ImportOutcome::Duplicate { stored_path }) => {
                        if let Some(path) =
                            stored_path.filter(|path| observed_paths.contains(path.as_str()))
                        {
                            progress.duplicate(report, &path, &album_id);
                        }
                        progress.duplicate(report, &candidate.relative_path, &album_id);
                    }
                    Err(error) => progress.persistence_failure(
                        report,
                        &candidate.relative_path,
                        &album_id,
                        &error,
                    ),
                }
            }
        }
        progress.publish();
    }
}

#[cfg(test)]
#[path = "../../tests/services/import/workflow.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/services/import/reconciliation_workflow.rs"]
mod reconciliation_tests;
