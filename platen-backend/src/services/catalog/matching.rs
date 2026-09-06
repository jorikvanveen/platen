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
mod tests {
    use super::*;
    use crate::{
        entity::{album_artist, artist},
        services::{
            catalog::{reconciliation::reconcile, utils::prepare_album},
            tidal::{ResolvedTidalSearchedAlbum, TidalAlbum, TidalArtist, TidalError},
        },
    };
    use sea_orm::{ActiveModelTrait, ConnectionTrait, QueryOrder, Set};
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::Semaphore;

    #[derive(Clone)]
    struct Record {
        album: TidalAlbum,
        artists: Vec<TidalArtist>,
        search_title: Option<String>,
        search_date: Option<String>,
        search_artists: Option<Vec<TidalArtist>>,
        failure: Option<&'static str>,
    }

    fn record(id: &str, title: &str, date: &str) -> Record {
        Record {
            album: TidalAlbum {
                id: id.into(),
                title: title.into(),
                release_date: Some(date.into()),
                cover_url: None,
                popularity: 0.0,
                r#type: "ALBUM".into(),
            },
            artists: vec![credit("primary", "AC/DC"), credit("guest", "Guest")],
            search_title: None,
            search_date: None,
            search_artists: None,
            failure: None,
        }
    }

    fn credit(id: &str, name: &str) -> TidalArtist {
        TidalArtist {
            id: id.into(),
            name: name.into(),
            profile_image_url: None,
        }
    }

    #[derive(Default)]
    struct FakeCatalog {
        records: Vec<Record>,
        search_fails: bool,
        metadata_calls: AtomicUsize,
        artist_calls: AtomicUsize,
        cover_calls: AtomicUsize,
        gates: HashMap<String, Arc<Semaphore>>,
        active: AtomicUsize,
        peak: AtomicUsize,
    }

    impl FakeCatalog {
        fn assert_no_detail_calls(&self) {
            assert_eq!(self.metadata_calls.load(Ordering::SeqCst), 0, "get_album");
            assert_eq!(
                self.artist_calls.load(Ordering::SeqCst),
                0,
                "get_album_artists"
            );
            assert_eq!(
                self.cover_calls.load(Ordering::SeqCst),
                0,
                "get_album_cover"
            );
        }
    }

    #[async_trait::async_trait]
    impl TidalCatalog for FakeCatalog {
        async fn find_album(
            &self,
            query: &str,
        ) -> Result<Vec<ResolvedTidalSearchedAlbum>, TidalError> {
            if self.search_fails {
                return Err(TidalError::UnexpectedResponse);
            }
            if let Some(gate) = self.gates.get(query) {
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(active, Ordering::SeqCst);
                gate.acquire().await.unwrap().forget();
                self.active.fetch_sub(1, Ordering::SeqCst);
            }
            Ok(self
                .records
                .iter()
                .map(|record| ResolvedTidalSearchedAlbum {
                    id: record.album.id.clone(),
                    title: record
                        .search_title
                        .clone()
                        .unwrap_or_else(|| record.album.title.clone()),
                    cover_url: record.album.cover_url.clone(),
                    release_date: record
                        .search_date
                        .clone()
                        .or_else(|| record.album.release_date.clone()),
                    popularity: record.album.popularity,
                    artists: record
                        .search_artists
                        .clone()
                        .unwrap_or_else(|| record.artists.clone()),
                    r#type: record.album.r#type.clone(),
                })
                .collect())
        }

        async fn get_album(&self, id: &str) -> Result<TidalAlbum, TidalError> {
            self.metadata_calls.fetch_add(1, Ordering::SeqCst);
            let record = self
                .records
                .iter()
                .find(|record| record.album.id == id)
                .unwrap();
            if record.failure == Some("metadata") {
                return Err(TidalError::UnexpectedResponse);
            }
            Ok(record.album.clone())
        }

        async fn get_album_artists(&self, id: &str) -> Result<Vec<TidalArtist>, TidalError> {
            self.artist_calls.fetch_add(1, Ordering::SeqCst);
            let record = self
                .records
                .iter()
                .find(|record| record.album.id == id)
                .unwrap();
            if record.failure == Some("credits") {
                return Err(TidalError::UnexpectedResponse);
            }
            Ok(record.artists.clone())
        }

        async fn get_album_cover(&self, id: &str) -> Result<Option<String>, TidalError> {
            self.cover_calls.fetch_add(1, Ordering::SeqCst);
            let record = self
                .records
                .iter()
                .find(|record| record.album.id == id)
                .unwrap();
            if record.failure == Some("cover") {
                return Err(TidalError::UnexpectedResponse);
            }
            Ok(None)
        }
    }

    fn candidate(path: &str, artist: &str, title: &str, year: Option<i32>) -> AlbumCandidate {
        AlbumCandidate {
            primary_artist: artist.into(),
            title: title.into(),
            release_year: year,
            relative_path: path.into(),
        }
    }

    fn report(candidates: Vec<AlbumCandidate>) -> DiscoveryReport {
        DiscoveryReport {
            resolved_root: Some("/resolved/music".into()),
            candidates,
            ..Default::default()
        }
    }

    async fn database() -> DatabaseConnection {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        <migration::Migrator as migration::MigratorTrait>::up(&db, None)
            .await
            .unwrap();
        db
    }

    async fn run(
        db: &DatabaseConnection,
        source: &dyn TidalCatalog,
        candidates: Vec<AlbumCandidate>,
    ) -> ScanSummary {
        let mut report = report(candidates);
        let mut summary = ScanSummary::from(&report);
        let unknowns = reconcile(db, &mut report, &mut summary).await.unwrap();
        let (scan, handle, _) = ActiveScan::new();
        scan.publish_summary(&summary).await;
        match_and_import(db, source, &report, unknowns, &mut summary, &scan).await;
        assert_eq!(handle.snapshot().await.summary, summary);
        summary
    }

    #[tokio::test]
    async fn conservative_identity_year_and_ordered_primary_artist() {
        for (artist, title, year, expected) in [
            (" ac - dc ", " a_b   (live) ", Some(2024), true),
            ("AC - DC", "A_B (Live)", None, true),
            ("AC/DC", "A:B (Live)", Some(2023), false),
            ("Guest", "A:B (Live)", None, false),
            ("ACDC", "A:B (Live)", None, false),
            ("AC/DC", "A:B", None, false),
            ("AC/DC", "A-B (Live)", None, false),
            ("AC/DC", "A:B (Deluxe)", None, false),
        ] {
            let source = FakeCatalog {
                records: vec![record("one", "A:B (Live)", "2024-02-29")],
                ..Default::default()
            };
            let result = match_candidate(
                &source,
                &report(vec![]),
                &candidate("Artist/Album", artist, title, year),
            )
            .await;
            assert_eq!(
                matches!(result, MatchOutcome::Unique(_)),
                expected,
                "{artist} {title} {year:?}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn dates_must_parse_and_match_in_search_metadata_only() {
        for (search_date, metadata_date, year, expected) in [
            ("2024", "2024", Some(2024), "unique"),
            ("2024-02", "2024-02-29", Some(2024), "unique"),
            ("2023", "2024", Some(2024), "unmatched"),
            ("2024", "2023", Some(2024), "unique"),
            ("2024-02-30", "2024", None, "failed"),
            ("2024", "2024-02-30", None, "unique"),
            ("", "2024", None, "failed"),
        ] {
            let mut album = record("one", "Title", metadata_date);
            album.search_date = Some(search_date.into());
            let source = FakeCatalog {
                records: vec![album],
                ..Default::default()
            };
            let outcome = match_candidate(
                &source,
                &report(vec![]),
                &candidate("Artist/Title", "AC/DC", "Title", year),
            )
            .await;
            assert_eq!(
                outcome_name(&outcome),
                expected,
                "{search_date} {metadata_date} {year:?}"
            );
            source.assert_no_detail_calls();
        }
    }

    fn outcome_name(outcome: &MatchOutcome) -> &'static str {
        match outcome {
            MatchOutcome::Unique(_) => "unique",
            MatchOutcome::Unmatched => "unmatched",
            MatchOutcome::Ambiguous(_) => "ambiguous",
            MatchOutcome::Failed => "failed",
        }
    }

    #[tokio::test]
    async fn uniqueness_requires_valid_search_metadata_and_distinct_ids() {
        let first = record("one", "Title", "2024");
        let later = record("two", "Title", "2025");
        let mut wrong_title = record("two", "Title (Deluxe)", "2024");
        wrong_title.search_title = Some("Title".into());
        let mut wrong_primary = record("two", "Title", "2024");
        wrong_primary.search_artists = Some(wrong_primary.artists.clone());
        wrong_primary.artists.reverse();
        let mut broken = record("two", "Title", "2024");
        broken.search_artists = Some(vec![credit("primary", "AC/DC"), credit("", "Guest")]);
        for (records, year, expected) in [
            (vec![], None, "unmatched"),
            (vec![first.clone(), first.clone()], None, "unique"),
            (vec![first.clone(), later.clone()], None, "ambiguous"),
            (vec![first.clone(), later], Some(2024), "unique"),
            (
                vec![first.clone(), record("two", "Title", "2024")],
                Some(2024),
                "ambiguous",
            ),
            (vec![first.clone(), wrong_title], None, "ambiguous"),
            (vec![first.clone(), wrong_primary], None, "ambiguous"),
            (vec![first, broken], None, "failed"),
        ] {
            let source = FakeCatalog {
                records,
                ..Default::default()
            };
            let outcome = match_candidate(
                &source,
                &report(vec![]),
                &candidate("Artist/Title", "AC/DC", "Title", year),
            )
            .await;
            assert_eq!(outcome_name(&outcome), expected);
            if let MatchOutcome::Ambiguous(ids) = outcome {
                assert_eq!(ids, vec!["one", "two"]);
            }
            source.assert_no_detail_calls();
        }
    }

    #[tokio::test]
    async fn invalid_search_metadata_and_candidate_failures_do_not_import() {
        for failure in [
            "search",
            "empty_credits",
            "blank_id",
            "blank_title",
            "missing_date",
            "invalid_date",
            "blank_primary_name",
            "blank_primary_id",
            "blank_guest_name",
            "blank_guest_id",
            "duplicate_artist_ids",
        ] {
            let mut album = record("one", "Title", "2024");
            let mut artists = album.artists.clone();
            match failure {
                "empty_credits" => artists.clear(),
                "blank_id" => album.album.id = " ".into(),
                "blank_title" => album.search_title = Some(" ".into()),
                "missing_date" => album.album.release_date = None,
                "invalid_date" => album.search_date = Some("2024-02-30".into()),
                "blank_primary_name" => artists[0].name = " ".into(),
                "blank_primary_id" => artists[0].id = " ".into(),
                "blank_guest_name" => artists[1].name = " ".into(),
                "blank_guest_id" => artists[1].id = " ".into(),
                "duplicate_artist_ids" => artists[1].id = artists[0].id.clone(),
                _ => {}
            }
            album.search_artists = Some(artists);
            let source = FakeCatalog {
                records: vec![album],
                search_fails: failure == "search",
                ..Default::default()
            };
            let db = database().await;
            let summary = run(
                &db,
                &source,
                vec![candidate("Artist/Title", "AC/DC", "Title", None)],
            )
            .await;
            assert_eq!(summary.albums_imported, 0, "{failure}");
            assert_eq!(summary.failures, 1, "{failure}");
            assert!(
                album::Entity::find().all(&db).await.unwrap().is_empty(),
                "{failure}"
            );
            assert_eq!(summary.skipped_directories, summary.failures);
            assert_eq!(summary.candidates_processed, 1);
            source.assert_no_detail_calls();
        }
    }

    #[tokio::test]
    async fn imports_preserve_search_data_without_any_detail_requests() {
        for failure in [None, Some("metadata"), Some("credits"), Some("cover")] {
            let db = database().await;
            let mut album = record("one", "Detail title", "1999");
            album.search_title = Some("Search title".into());
            album.search_date = Some("2024-02-29".into());
            album.album.cover_url = Some("https://example.com/search-cover.jpg".into());
            album.album.popularity = 0.75;
            album.album.r#type = "EP".into();
            let mut primary = credit("search-primary", "AC/DC");
            primary.profile_image_url = Some("https://example.com/primary.jpg".into());
            let mut guest = credit("search-guest", "Search guest");
            guest.profile_image_url = Some("https://example.com/guest.jpg".into());
            let search_artists = vec![primary, guest];
            album.search_artists = Some(search_artists.clone());
            album.artists = vec![credit("detail-primary", "Different artist")];
            album.failure = failure;
            let source = FakeCatalog {
                records: vec![album],
                ..Default::default()
            };
            let candidate = candidate("Artist/Search title", "AC/DC", "Search title", Some(2024));
            let outcome = match_candidate(&source, &report(vec![]), &candidate).await;
            source.assert_no_detail_calls();
            let MatchOutcome::Unique(prepared) = outcome else {
                panic!("Expected unique search match with {failure:?}, got {outcome:?}");
            };
            assert_eq!(prepared.album().popularity, 0.75);
            assert_eq!(prepared.album().release_date.as_deref(), Some("2024-02-29"));

            let summary = run(&db, &source, vec![candidate]).await;
            source.assert_no_detail_calls();
            assert_eq!(summary.albums_imported, 1, "{failure:?}");
            assert_eq!(summary.failures, 0);
            assert_eq!(summary.skipped_directories, 0);
            let stored = album::Entity::find_by_id("one")
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.title, "Search title");
            assert_eq!(stored.album_type.as_deref(), Some("EP"));
            assert_eq!(stored.release_year, 2024);
            assert_eq!(stored.release_month, Some(2));
            assert_eq!(stored.release_day, Some(29));
            assert_eq!(
                stored.cover_url.as_deref(),
                Some("https://example.com/search-cover.jpg")
            );
            assert_eq!(stored.relative_path.as_deref(), Some("Artist/Search title"));
            let credits = album_artist::Entity::find()
                .filter(album_artist::Column::AlbumId.eq("one"))
                .order_by_asc(album_artist::Column::Position)
                .all(&db)
                .await
                .unwrap();
            assert_eq!(credits.len(), search_artists.len());
            for (position, (credit, expected)) in credits.iter().zip(&search_artists).enumerate() {
                assert_eq!(credit.artist_id, expected.id);
                assert_eq!(credit.position, position as i32);
                let stored_artist = artist::Entity::find_by_id(&credit.artist_id)
                    .one(&db)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(stored_artist.name, expected.name);
                assert_eq!(stored_artist.profile_image_url, expected.profile_image_url);
            }
            assert_eq!(artist::Entity::find().all(&db).await.unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn duplicate_locations_skip_all_copies_in_any_order() {
        for stored_path in [None, Some("Old/Title")] {
            for reverse in [false, true] {
                let db = database().await;
                if let Some(path) = stored_path {
                    album::ActiveModel {
                        id: Set("one".into()),
                        title: Set("Old title".into()),
                        release_year: Set(1999),
                        relative_path: Set(Some(path.into())),
                        ..Default::default()
                    }
                    .insert(&db)
                    .await
                    .unwrap();
                }
                let source = FakeCatalog {
                    records: vec![record("one", "Title", "2024")],
                    ..Default::default()
                };
                let mut candidates = vec![
                    candidate("Artist/Title", "AC/DC", "Title", None),
                    candidate("Artist/Title (2024)", "AC/DC", "Title", Some(2024)),
                ];
                if let Some(path) = stored_path {
                    candidates.push(candidate(path, "Old", "Title", None));
                }
                if reverse {
                    candidates.reverse();
                }
                let summary = run(&db, &source, candidates).await;
                assert_eq!(
                    summary.duplicate_locations,
                    2 + usize::from(stored_path.is_some())
                );
                assert_eq!(summary.skipped_directories, summary.duplicate_locations);
                assert_eq!(summary.albums_imported + summary.locations_attached, 0);
                let stored = album::Entity::find_by_id("one").one(&db).await.unwrap();
                assert_eq!(
                    stored
                        .as_ref()
                        .and_then(|album| album.relative_path.as_deref()),
                    stored_path
                );
                assert_eq!(stored.is_some(), stored_path.is_some());
            }
        }
    }

    #[tokio::test]
    async fn a_tidal_alias_does_not_double_count_known_duplicate_paths() {
        let db = database().await;
        let original = FakeCatalog {
            records: vec![record("one", "Old", "2024")],
            ..Default::default()
        };
        persist_album(
            &db,
            prepare_album(&original, "one").await.unwrap(),
            Some("Artist/Old".into()),
        )
        .await
        .unwrap();
        let source = FakeCatalog {
            records: vec![record("one", "New", "2024")],
            ..Default::default()
        };
        let summary = run(
            &db,
            &source,
            vec![
                candidate("Artist/Old", "AC/DC", "Old", None),
                candidate("Artist/Old (2024)", "AC/DC", "Old", Some(2024)),
                candidate("Artist/New", "AC/DC", "New", None),
            ],
        )
        .await;
        assert_eq!(summary.candidates_processed, 3);
        assert_eq!(summary.duplicate_locations, 3);
        assert_eq!(summary.skipped_directories, 3);
        assert_eq!(summary.unchanged_locations, 1);
        assert_eq!(summary.albums_imported + summary.locations_attached, 0);
        let stored = album::Entity::find_by_id("one")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.title, "Old");
        assert_eq!(stored.relative_path.as_deref(), Some("Artist/Old"));
    }

    #[tokio::test]
    async fn known_duplicates_without_a_stored_location_block_every_tidal_alias() {
        for stored_path in [None, Some("Missing/Old")] {
            for reverse in [false, true] {
                for alias_count in [1, 2] {
                    let db = database().await;
                    let original = FakeCatalog {
                        records: vec![record("one", "Old", "2024")],
                        ..Default::default()
                    };
                    persist_album(
                        &db,
                        prepare_album(&original, "one").await.unwrap(),
                        stored_path.map(str::to_owned),
                    )
                    .await
                    .unwrap();
                    let source = FakeCatalog {
                        records: vec![record("one", "New", "2024")],
                        ..Default::default()
                    };
                    let mut candidates = vec![
                        candidate("Artist/Old", "AC/DC", "Old", None),
                        candidate("Artist/Old (2024)", "AC/DC", "Old", Some(2024)),
                        candidate("Artist/New", "AC/DC", "New", None),
                    ];
                    if alias_count == 2 {
                        candidates.push(candidate("Artist/New (2024)", "AC/DC", "New", Some(2024)));
                    }
                    if reverse {
                        candidates.reverse();
                    }
                    let summary = run(&db, &source, candidates).await;
                    assert_eq!(summary.candidates_processed, 2 + alias_count);
                    assert_eq!(summary.duplicate_locations, 2 + alias_count);
                    assert_eq!(summary.skipped_directories, 2 + alias_count);
                    assert_eq!(
                        summary.albums_imported
                            + summary.locations_attached
                            + summary.locations_changed,
                        0
                    );
                    assert_eq!(
                        summary.locations_cleared,
                        usize::from(stored_path.is_some())
                    );
                    assert_eq!(
                        summary.failures + summary.unmatched_candidates + summary.ambiguous_matches,
                        0
                    );
                    let stored = album::Entity::find_by_id("one")
                        .one(&db)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(stored.title, "Old");
                    assert_eq!(stored.relative_path, None);
                }
            }
        }
    }

    #[tokio::test]
    async fn existing_ids_attach_without_refresh_and_imports_keep_credit_order() {
        let db = database().await;
        album::ActiveModel {
            id: Set("existing".into()),
            title: Set("Keep metadata".into()),
            release_year: Set(1999),
            cover_url: Set(Some("old-cover".into())),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
        let before = album::Entity::find_by_id("existing")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let source = FakeCatalog {
            records: vec![
                record("existing", "Title", "2024"),
                record("new", "New", "2024-02-29"),
            ],
            ..Default::default()
        };
        let candidates = vec![
            candidate("Artist/Title", "AC/DC", "Title", None),
            candidate("Artist/New", "AC/DC", "New", None),
        ];
        let summary = run(&db, &source, candidates.clone()).await;
        assert_eq!(summary.locations_attached, 1);
        assert_eq!(summary.albums_imported, 1);
        let mut after = album::Entity::find_by_id("existing")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.relative_path.as_deref(), Some("Artist/Title"));
        after.relative_path = None;
        assert_eq!(before, after);
        let credits = album_artist::Entity::find()
            .order_by_asc(album_artist::Column::Position)
            .all(&db)
            .await
            .unwrap();
        assert_eq!(
            credits
                .iter()
                .map(|credit| (
                    credit.album_id.as_str(),
                    credit.artist_id.as_str(),
                    credit.position
                ))
                .collect::<Vec<_>>(),
            vec![("new", "primary", 0), ("new", "guest", 1)]
        );
        source.assert_no_detail_calls();
        let second = run(&db, &source, candidates).await;
        assert_eq!(second.unchanged_locations, 2);
        assert_eq!(second.albums_imported + second.locations_attached, 0);
        source.assert_no_detail_calls();
    }

    #[tokio::test]
    async fn failed_import_rolls_back_independently_and_occupied_paths_are_skipped() {
        let db = database().await;
        db.execute_unprepared("CREATE TRIGGER fail_credit BEFORE INSERT ON album_artist WHEN NEW.album_id = 'bad' BEGIN SELECT RAISE(ABORT, 'test credit failure'); END").await.unwrap();
        let source = FakeCatalog {
            records: vec![record("bad", "Bad", "2024"), record("good", "Good", "2024")],
            ..Default::default()
        };
        let summary = run(
            &db,
            &source,
            vec![
                candidate("Artist/Bad", "AC/DC", "Bad", None),
                candidate("Artist/Good", "AC/DC", "Good", None),
            ],
        )
        .await;
        assert_eq!(summary.failures, 1);
        assert_eq!(summary.albums_imported, 1);
        assert!(
            album::Entity::find_by_id("bad")
                .one(&db)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(artist::Entity::find().all(&db).await.unwrap().len(), 2);
        let prepared = prepare_album(&source, "bad").await.unwrap();
        let mut summary = ScanSummary::default();
        import_match(
            &db,
            &report(vec![]),
            &candidate("Artist/Good", "AC/DC", "Bad", None),
            prepared,
            &mut summary,
        )
        .await
        .unwrap();
        assert_eq!(summary.duplicate_locations, 1);
        assert_eq!(summary.skipped_directories, 1);
        assert_eq!(summary.failures, 0);
        assert!(
            album::Entity::find_by_id("bad")
                .one(&db)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn matching_is_bounded_publishes_each_completion_and_defers_all_imports() {
        let db = database().await;
        let first = Arc::new(Semaphore::new(0));
        let second = Arc::new(Semaphore::new(0));
        let third = Arc::new(Semaphore::new(0));
        let source = Arc::new(FakeCatalog {
            records: vec![
                record("one", "One", "2024"),
                record("two", "Two", "2024"),
                record("three", "Three", "2024"),
            ],
            gates: HashMap::from([
                ("AC/DC One".into(), first.clone()),
                ("AC/DC Two".into(), second.clone()),
                ("AC/DC Three".into(), third.clone()),
            ]),
            ..Default::default()
        });
        let report = report(vec![
            candidate("Artist/One", "AC/DC", "One", None),
            candidate("Artist/Two", "AC/DC", "Two", None),
            candidate("Artist/Three", "AC/DC", "Three", None),
        ]);
        let (scan, handle, _) = ActiveScan::new();
        let task = tokio::spawn({
            let db = db.clone();
            let source = source.clone();
            async move {
                let mut summary = ScanSummary::from(&report);
                scan.publish_summary(&summary).await;
                match_and_import(
                    &db,
                    source.as_ref(),
                    &report,
                    report.candidates.clone(),
                    &mut summary,
                    &scan,
                )
                .await;
                summary
            }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while source.active.load(Ordering::SeqCst) != 2 {
                tokio::task::yield_now().await;
            }
            assert_eq!(handle.snapshot().await.summary.candidates_processed, 0);
            first.add_permits(1);
            while handle.snapshot().await.summary.candidates_processed != 1 {
                tokio::task::yield_now().await;
            }
            assert!(album::Entity::find().all(&db).await.unwrap().is_empty());
            third.add_permits(1);
            while handle.snapshot().await.summary.candidates_processed != 2 {
                tokio::task::yield_now().await;
            }
            assert!(album::Entity::find().all(&db).await.unwrap().is_empty());
            second.add_permits(1);
        })
        .await
        .unwrap();
        let summary = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(summary.candidates_processed, 3);
        assert_eq!(summary.albums_imported, 3);
        assert_eq!(source.peak.load(Ordering::SeqCst), 2);
        assert_eq!(handle.snapshot().await.summary, summary);
    }
}
