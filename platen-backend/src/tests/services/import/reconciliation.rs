use super::*;

fn candidate(path: &str, artist: &str, title: &str, year: Option<i32>) -> AlbumCandidate {
    AlbumCandidate {
        relative_path: path.into(),
        primary_artist: artist.into(),
        title: title.into(),
        release_year: year,
    }
}

#[test]
fn plans_are_deterministic_and_do_not_mutate_their_inputs() {
    let catalog = vec![
        CatalogAlbum {
            id: "known".into(),
            primary_artist: Some("Artist".into()),
            title: "Title".into(),
            release_year: 2024,
            relative_path: Some("Old/Title".into()),
        },
        CatalogAlbum {
            id: "missing".into(),
            primary_artist: None,
            title: "Missing".into(),
            release_year: 2024,
            relative_path: Some("Old/Missing".into()),
        },
    ];
    let candidates = vec![
        candidate("Artist/Title (2024)", "Artist", "Title", Some(2024)),
        candidate("Artist/Unknown", "Artist", "Unknown", None),
    ];
    let plan = reconcile(&candidates, &catalog);
    assert_eq!(
        plan.locations,
        vec![
            LocationUpdate {
                album_id: "known".into(),
                previous_path: Some("Old/Title".into()),
                relative_path: Some("Artist/Title (2024)".into())
            },
            LocationUpdate {
                album_id: "missing".into(),
                previous_path: Some("Old/Missing".into()),
                relative_path: None
            },
        ]
    );
    assert_eq!(plan.unknown_candidates, vec![candidates[1].clone()]);
    assert_eq!(catalog[0].relative_path.as_deref(), Some("Old/Title"));
    assert_eq!(
        plan,
        reconcile(
            &candidates.into_iter().rev().collect::<Vec<_>>(),
            &catalog.into_iter().rev().collect::<Vec<_>>()
        )
    );
}

#[test]
fn duplicate_evidence_belongs_to_the_plan_even_without_a_stored_location() {
    let catalog = vec![CatalogAlbum {
        id: "known".into(),
        primary_artist: Some("Artist".into()),
        title: "Title".into(),
        release_year: 2024,
        relative_path: None,
    }];
    let candidates = vec![
        candidate("Artist/Title", "Artist", "Title", None),
        candidate("Artist/Title (2024)", "Artist", "Title", Some(2024)),
    ];
    let plan = reconcile(&candidates, &catalog);
    assert!(plan.locations.is_empty());
    assert!(plan.unknown_candidates.is_empty());
    assert_eq!(
        plan.duplicate_paths_by_album_id["known"],
        vec!["Artist/Title", "Artist/Title (2024)"]
    );
    assert_eq!(
        plan,
        reconcile(&candidates.into_iter().rev().collect::<Vec<_>>(), &catalog)
    );
}

#[test]
fn location_decisions_require_a_unique_candidate_unless_the_old_path_is_observed() {
    let first = candidate("Artist/Title", "Artist", "Title", None);
    let second = candidate("Artist/Title (2024)", "Artist", "Title", Some(2024));
    let candidates = [&first, &second];
    for (old_path, observed, expected) in [
        (
            None,
            false,
            [
                LocationDecision::NoLocation,
                LocationDecision::Attach("Artist/Title"),
                LocationDecision::NoLocation,
            ],
        ),
        (
            Some("Old/Title"),
            false,
            [
                LocationDecision::Clear,
                LocationDecision::Change("Artist/Title"),
                LocationDecision::Clear,
            ],
        ),
        (
            Some("Old/Title"),
            true,
            [
                LocationDecision::Keep("Old/Title"),
                LocationDecision::Keep("Old/Title"),
                LocationDecision::Keep("Old/Title"),
            ],
        ),
    ] {
        for (candidate_count, expected) in expected.into_iter().enumerate() {
            assert_eq!(
                decide_location(old_path, observed, &candidates[..candidate_count]),
                expected,
                "old_path={old_path:?}, observed={observed}, candidates={candidate_count}"
            );
        }
    }
}
