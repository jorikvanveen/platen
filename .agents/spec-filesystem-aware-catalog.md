# Filesystem-aware Catalog

## Problem Statement

Platen records whether it completed a Download, but it does not know whether an Album's audio is actually present in the Music directory. Files copied into the Music directory outside Platen never enter the Catalog, moved Albums retain stale state, and deleted or inaccessible directories still appear downloaded. A user with an existing collection must add each Album from Tidal by hand even though the Music directory already identifies its Primary artist, title, and often its release year.

## Solution

Add a user-triggered, read-only Music directory scan. The scan discovers Albums from Platen's two-level filesystem layout, reconciles locations for existing Catalog Albums, searches Tidal for unknown directories, and automatically imports only unique high-confidence matches. Tidal remains the identity, metadata, and Album credit authority.

Replace the historical `downloaded` flag with an optional Album location relative to the configured Music directory. A location means Platen can currently observe the Album's audio, regardless of whether Antra or another source supplied it. A successful scan clears locations Platen can no longer see but never deletes Catalog Albums.

Expose the scan as one in-memory background operation under the Catalog API and provide an Import page that starts the scan, polls progress, and displays aggregate outcomes. Per-directory ambiguity and failures go to structured logs. User-driven ambiguity resolution will be designed separately.

## User Stories

1. As a user with an existing music collection, I want Platen to scan the configured Music directory, so that I do not have to add every Album manually.
2. As a user, I want the scan to use the Music directory already configured for Downloads, so that I do not expose arbitrary server paths through the API.
3. As a user, I want scans to run only when requested, so that filesystem and Tidal work happens at a predictable time.
4. As a user, I want Platen to recognize the same Primary artist and Album directory layout it writes, so that downloaded and externally copied Albums follow one rule.
5. As a user, I want Platen to find audio inside disc subdirectories, so that multi-disc Albums count as downloaded.
6. As a user, I want artwork-only and empty directories ignored, so that they do not create false Albums.
7. As a user, I want common lossless and lossy audio formats recognized without reading tags, so that discovery stays independent of embedded metadata quality.
8. As a user, I want scans to avoid descendant symbolic links, so that they cannot escape the Music directory or loop through the filesystem.
9. As an operator whose Music directory is itself a symbolic link, I want Platen to resolve the configured root once, so that relocated and mounted collections still work.
10. As a user, I want Platen to leave my files untouched during a scan, so that an incorrect match cannot rename, move, overwrite, or delete audio.
11. As a user, I want a Catalog Album to be downloaded exactly when it has an observed location, so that boolean history cannot disagree with the filesystem.
12. As a user, I want Album locations stored relative to the Music directory, so that moving the configured root does not invalidate every Catalog row.
13. As a user, I want API responses to expose the relative Album location, so that clients use the same presence state as the backend.
14. As a user upgrading Platen, I want old boolean values discarded rather than converted into invented paths, so that the first scan establishes truthful state.
15. As a user, I want a scan to attach a location to an existing Catalog Album, so that Albums added from Tidal become downloaded when matching audio is present.
16. As a user, I want a scan to update an Album after its directory moves, so that Platen records the location it can currently observe.
17. As a user, I want a missing or inaccessible Album directory to clear the Album location without deleting the Album, so that the Catalog remains intact while downloaded state reflects visibility.
18. As a user, I want permission failures reported, so that I can tell the difference between a clean scan and audio Platen could not access.
19. As a user, I want unknown directories matched against Tidal by Primary artist, title, and optional release year, so that filesystem names can seed complete Catalog metadata.
20. As a user, I want Tidal Artist credits kept in relationship order, so that the first credit reliably identifies the Primary artist.
21. As a user, I want conservative name comparison, so that case and harmless filesystem formatting differences match without silently discarding meaningful edition labels or punctuation.
22. As a user, I want a directory year to agree with Tidal, so that similarly named editions from other years are not imported automatically.
23. As a user, I want an Album without a year in its directory to import only when Artist and title identify one result, so that missing information does not turn into guessing.
24. As a user, I want only one verified Tidal result to import automatically, so that Platen does not repeat the retired first-search-result behavior.
25. As a user, I want ambiguous and unmatched directories skipped, so that uncertain identity never enters the Catalog.
26. As a user, I want two directories matching the same Tidal Album both skipped, so that filesystem traversal order cannot select one copy arbitrarily.
27. As a user, I want imported Albums to use normal Tidal metadata and ordered Album credits, so that filesystem discovery does not create weaker Catalog rows.
28. As a user, I want missing cover art tolerated but invalid release dates or missing credits rejected, so that imported Albums follow the same validity rules as manually added Albums.
29. As a user, I want one scan to continue after an individual Tidal or database failure, so that one problematic directory does not discard successful imports.
30. As a user, I want at most two filesystem candidates matched through Tidal concurrently, so that initial imports make progress without provoking avoidable rate limits.
31. As a user, I want a scan request to return immediately, so that a large collection does not hold an HTTP request open for minutes.
32. As a user, I want to see whether Platen is scanning the filesystem or matching Tidal candidates, so that progress is understandable.
33. As a user, I want a second scan request rejected while one is active, so that scans do not duplicate work or race over Album locations.
34. As a user, I want the latest scan summary available after browser navigation, so that closing the Import page does not hide a running scan.
35. As a user, I want scan summaries to report discovered, imported, attached, changed, unchanged, cleared, ambiguous, duplicate, skipped, failed, and filesystem-error counts, so that I can assess the result without reading server logs.
36. As an operator, I want skipped directories logged with named reason and path fields, so that I can search logs for a specific failure class.
37. As a user, I want the Import page linked from the main header, so that filesystem discovery is a first-class Catalog action.
38. As a user, I want Artist pages to keep showing whether an Album is downloaded, so that replacing the boolean does not remove the existing affordance.
39. As a user, I want the Album location available as secondary UI information, so that I can see which directory Platen associates with the Album.
40. As a user, I want Download requests for known or untracked existing destinations rejected, so that Platen never merges with or overwrites filesystem state.
41. As a user, I want Downloads published atomically, so that a failed transfer cannot leave a partial Album directory that a scan treats as complete.
42. As a user, I want a successful Download to set the Album location only after publication, so that database state never gets ahead of filesystem placement.
43. As a user, I want failed Downloads to clean their staging directory when possible, so that temporary files do not accumulate during normal failures.
44. As a user, I want scan traversal and Download publication coordinated, so that a scan cannot observe or clear state halfway through a Download.
45. As an operator, I want a process restart to leave imported Albums intact even though scan status is in memory, so that completed work is not coupled to job persistence.

## Implementation Decisions

- The configured Music directory is the only scan root. The API never accepts a server path.
- A scan is manual, read-only, and non-cancellable. Platen does not watch the filesystem.
- The supported shape is one Primary artist directory containing Album directories. An Album directory name contains its title and may end in a four-digit release year in parentheses.
- An Album directory qualifies when at least one recognized audio file exists anywhere below it. Recognized extensions are FLAC, MP3, M4A, AAC, OGG, Opus, WAV, AIFF, AIF, and ALAC, compared case-insensitively.
- The scan resolves a symbolic-link Music directory root once. It does not follow links below that root. Non-UTF-8 directory names are logged and skipped.
- One shared filesystem-safe component function handles Primary artist and Album title names. Downloads and matching use the same conversion. Comparison ignores case, trims outer whitespace, and collapses repeated whitespace. It does not remove punctuation, edition labels, or other words.
- The Album schema replaces `downloaded` with nullable `relative_path`. Non-null paths are unique, relative, slash-separated, case-preserving, non-empty, and cannot contain `..`.
- Existing rows receive a null location during migration. No path is inferred from the old boolean.
- Album DTOs and generated client bindings remove `downloaded` and expose `relative_path`.
- A non-null Album location is the only representation of downloaded state. The source of audio is not stored.
- A scan inventories every visible qualifying Album location. Any stored path absent from that inventory is cleared, including paths hidden by permissions, missing mounts, unreadable directories, or filesystem metadata errors. Catalog rows are never deleted by a scan.
- Filesystem errors increment summary counters and emit structured logs. They do not prevent the scan from completing or location clearing.
- Existing Albums attach the discovered location, keep an identical location, or move to a newly matched location when the old one is absent. If old and new locations both contain audio, neither changes.
- Tidal remains the identity, metadata, release-date, artwork, and ordered Album credit authority.
- Tidal Album search resolution must preserve the Artist ID order from the Album relationship rather than the order of top-level included resources. The first Artist is Primary.
- Matching uses the filesystem-safe Primary artist and title plus an optional year. A supplied year must equal the Tidal release year. Without a year, Artist and title must still produce exactly one verified match.
- Matching examines the results returned by the existing Tidal search operation. Search pagination is not added.
- At most two filesystem candidates perform Tidal matching concurrently.
- A unique high-confidence match imports automatically. The scan does not wait for per-Album user acceptance.
- New Albums use the same creation service and validity rules as manual Tidal addition. Missing cover art is allowed. Missing ordered credits or an invalid release date causes a logged skip.
- Existing Album metadata and credits are not refreshed during filesystem scanning.
- The scan resolves all candidates before mutating new or moved Album matches, groups results by Tidal Album ID, and discards every group containing more than one location. Remaining Albums commit independently.
- Individual Tidal, validation, and persistence failures do not roll back other imported Albums. A later scan is idempotent.
- One background scan may run in the process. `POST /catalog/scan` starts it and returns `202 Accepted`. `GET /catalog/scan` returns the active scan or latest summary. A concurrent start returns `409 Conflict` with active status.
- Scan states are `scanning`, `matching`, `completed`, and `failed`. Active and latest state live in memory and reset on process restart.
- The summary reports Album directories found, candidates processed and total, Albums imported, locations attached, locations changed, unchanged locations, locations cleared, unmatched candidates, ambiguous matches, duplicate locations, skipped directories, Tidal or database failures, and filesystem errors.
- Detailed outcomes use structured tracing fields such as `reason`, `path`, Tidal Album IDs, and `os_error`. Filesystem diagnostics log absolute paths under the resolved Music directory root. If root resolution fails, they log the configured root as supplied. Diagnostic paths do not need to be relative; stored Album locations and API location fields remain relative. Detailed outcomes are not stored in the database or returned as an unbounded result list.
- The frontend adds an Import page and header link. It starts scans, polls status, displays phase and progress, and renders aggregate results. It has no ambiguity-resolution controls.
- Artist pages derive downloaded state from a non-null relative path and may show the location as secondary metadata or a tooltip.
- Downloads reject unknown Albums with `404 Not Found`. They reject a known Album location or any existing computed destination with `409 Conflict`, even when the destination is empty or lacks recognized audio.
- Downloads stage in an opaque child of a reserved `.platen-staging` directory on the Music directory filesystem. Scans always ignore that directory.
- A completed Download publishes by atomic rename and sets the relative Album location only after publication. Failed Downloads remove their own staging directory when possible. Hard-stop leftovers remain ignored.
- The scan and Download worker share a filesystem-operation lock during traversal and stale-location reconciliation. Tidal matching does not hold the lock.
- The filesystem scan creates a new accepted ADR that supersedes only the parts of the media-server boundary decision which prohibited filesystem-derived Catalog creation. Media servers remain outside Platen's boundary.
- Generated SeaORM entities and TypeScript bindings are regenerated with repository scripts and never edited manually.

## Testing Decisions

- The primary behavioral seam is the real HTTP router with the real scan coordinator, a real temporary Music directory, migrated in-memory SQLite, and one fake Tidal Catalog source. Tests start scans through HTTP, poll status through HTTP, and verify both responses and persisted Catalog state.
- The central router test covers asynchronous start, single-scan exclusion, filesystem discovery, existing location reconciliation, automatic unique Tidal import, duplicate and ambiguous skips, relative-path persistence, ordered credits, and summary counters.
- A gated fake Tidal source proves that `POST` returns before matching finishes and that status remains observable while work is blocked.
- Introduce one narrow Tidal Catalog abstraction for search and the metadata operations required by shared Album creation. Do not abstract the filesystem, SeaORM database, Tokio executor, or clock.
- Filesystem discovery tests use real temporary directories. They cover optional year parsing, titles containing parentheses, recursive audio discovery, extension casing, malformed layouts, empty directories, symbolic links, reserved staging paths, relative output, and non-UTF-8 names.
- Reconciliation tests accept a discovery report and use migrated SQLite. They verify that every unobserved path clears, including a path associated with a permission diagnostic, while observed paths attach or change correctly.
- Matching tests are table-driven. They cover normalization, release-year agreement, yearless uniqueness, multiple editions, ordered multi-Artist credits, no results, ambiguous results, and per-candidate Tidal failures.
- Tidal response fixture tests reverse top-level included Artist order and verify that relationship order still determines Primary artist.
- Migration and serialization tests verify that new and migrated Albums default to no location, API JSON includes `relative_path`, API JSON excludes `downloaded`, and generated TypeScript has the same shape.
- Atomic Download tests use the existing Downloader abstraction, migrated SQLite, and a temporary Music directory. They verify staging, publication, collision rejection, cleanup, relative-path persistence, and absence of partial final directories after failure.
- Frontend request and polling behavior lives in a small plain TypeScript helper with injected `fetch`. Vitest covers GET, POST, polling termination, conflict and error states, and summary decoding. No browser or Svelte component-testing dependency is added.
- Existing backend tests, frontend unit tests, frontend type checks, and production builds remain green.
- Tests assert externally visible behavior and persisted state rather than private helper call order.

## Out of Scope

- Continuous filesystem watching or automatic startup scans
- Scanning arbitrary request-provided server paths
- Audio-tag or file-content metadata parsing
- Media-server integration, identifiers, reconciliation, or scan requests
- MusicBrainz lookup or identifiers
- Tidal search pagination
- User review or resolution of ambiguous, unmatched, or duplicate locations
- Tracking whether audio came from Antra or another source
- More than one filesystem location per Album
- Moving, renaming, deleting, repairing, or reorganizing files during a scan
- Persisted scan jobs, scan history, restart recovery, or cancellation
- Refreshing metadata or Album credits for existing Catalog Albums
- Track-level Catalog entities or credits
- Multiple concurrent scans
- More than two concurrent Tidal filesystem matches
- A frontend browser-testing stack

## Further Notes

This decision intentionally changes the boundary recorded by the media-server ADR. Filesystem directories may now lead to Catalog creation, but only after Tidal supplies a unique identity and complete Album metadata. Raw files and media-server records still do not become independent identity authorities.

A later specification will cover user resolution of ambiguous, unmatched, and duplicate locations. This version preserves enough detail in structured logs to diagnose those cases without creating a persistent review queue prematurely.
