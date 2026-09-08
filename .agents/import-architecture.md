# Import architecture

Import is the UI name for a user-requested Music directory scan. This redesign changes ownership and dependencies, not catalog identity or matching policy.

## Backend

`services/import` owns the scan use case. Shared album preparation, release dates, and catalog insertion stay in `services/catalog`. HTTP routes map domain snapshots to DTOs.

| Module | Responsibility | Must not own |
| --- | --- | --- |
| `discovery` | Read the directory tree and return candidates, diagnostics, and discovery progress | Catalog records, Tidal calls, scan state |
| `reconciliation` | Build a location plan from candidates and a catalog snapshot | I/O, persistence, counters |
| `matching` | Search Tidal and validate whether a candidate has exactly one match | Database writes, discovery reports, scan state |
| `repository` | Read catalog records and apply location changes and imports transactionally | Filesystem traversal, Tidal calls, progress |
| `workflow` | Sequence stages, limit searches to two, collect all matches before import, count outcomes once | HTTP types, shared mutable job state |
| `coordinator` | Admit one job, run it in the background, retain the latest snapshot, report unexpected worker termination | Matching and persistence policy |

The workflow owns its progress. It publishes complete snapshots through a callback. The coordinator retains snapshots in a watch channel, so readers cannot edit workflow state. Discovery publishes only filesystem counters through its own callback.

Reconciliation returns explicit location updates, unknown candidates, ambiguous candidates, and duplicate paths grouped by album ID. It never changes the discovery report. The workflow tracks duplicate paths in one set so later Tidal aliases cannot count known duplicates twice.

The repository checks path occupancy and album location inside each import transaction. Metadata and ordered credits enter the catalog together. Existing metadata is never refreshed. Planned location updates compare the stored location against the location used to make the plan, so they cannot overwrite a newer location.

The Music directory lock covers discovery and catalog reconciliation. Tidal searches do not hold it. Independent failed imports do not roll back successful imports.

## Frontend

`catalogScan.ts` handles HTTP requests and DTO validation. `importController.ts` owns request state and polling lifetime. The Svelte page subscribes to controller state and renders it. A pending start blocks duplicate submissions before the server responds. Leaving the page aborts requests and polling delays. A polling failure preserves the last scan and allows progress polling to resume without starting a new scan.

## Compatibility

- Keep GET and POST `/catalog/scan`, including 202 and 409 responses, existing DTO fields, and phase names.
- Keep stored-path precedence, primary-artist/title normalization, optional year matching, search-result-only metadata, and rejection of incomplete alternatives.
- Keep duplicate handling independent of filesystem and Tidal completion order.
- Keep read-only filesystem behavior, supported audio extensions, recursive disc folders, and symlink/staging exclusions.
- Keep clearing unobserved locations, including inaccessible or missing roots, as required by ADR-0007 and existing tests.
- Keep in-memory latest-run status. No migration, persistent queue, event bus, new package, or generic repository framework.

## Validation

Existing behavioral tests moved with their responsibilities. New tests cover deterministic pure plans, stale location writes, transactional conflict checks, worker failure/restart, simultaneous starts, and frontend start/poll/disposal races.

Verified after implementation:

- `cargo test --offline -- --skip export_bindings` passed all 149 backend tests.
- `CARGO_NET_OFFLINE=true sh scripts/generate-types.sh` passed all 19 binding exporters. Generated DTOs are unchanged.
- `cargo build --offline` passed.
- `npm test -- --reporter=dot` passed all 99 frontend tests.
- `npm run check` reported no errors or warnings.
- `npm run build` passed.
- Strict Clippy found existing `result_large_err` and `needless_lifetimes` warnings in `config.rs` and `services/tidal.rs`. With only those two lint categories allowed, `cargo clippy --offline --all-targets -- -D warnings -A clippy::result_large_err -A clippy::needless_lifetimes` passed. Those unrelated files remain unchanged.

Tests use temporary directories, isolated SQLite databases, and fake Tidal services. No live scan or external-service validation ran.
