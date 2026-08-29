# 0009: Remove media-server reconciliation

**Status:** ready-for-agent

## Problem Statement

Platen currently treats Jellyfin as part of its domain. It reads Jellyfin's
catalog, imports and links Albums, stores Jellyfin identifiers, exposes import
and status routes, and requires Jellyfin configuration at startup. This couples
Platen's catalog to the state of one media server even though Platen already
owns the catalog and the layout of its music directory.

The import also supplies the only live use of MusicBrainz. Removing the import
would leave a dead MusicBrainz client and identifiers that Platen can no longer
populate or use. Both integrations should leave together rather than survive as
misleading partial features.

Platen should write a music directory that Jellyfin and other media servers can
scan, using the directory behavior that already works. Media servers are
filesystem consumers outside Platen's boundary. Their catalog state must not
create, delete, or modify Platen's catalog.

## Solution

Remove Jellyfin reconciliation and MusicBrainz support completely. Platen will
use Tidal to identify Albums and Artists and Antra to download audio. Albums
will enter the catalog only through an explicit Tidal-backed add action.

Keep the current music-directory layout and download behavior unchanged. A
media server may scan the resulting files on its own schedule, but Platen will
not query a media server, store its identifiers, reconcile against its catalog,
or request a scan.

Assume a clean database. Rewrite the initial schema migration so new databases
never contain Jellyfin identifiers, MusicBrainz identifiers, or album resolution
provenance. Do not add an upgrade migration or compatibility path for existing
databases.

## User Stories

1. As a Platen user, I want my catalog to represent Albums I explicitly add, so that a media server cannot silently change what Platen knows about.
2. As a Platen user, I want to add an Album through Tidal, so that its Tidal identity and credited Artists remain the basis of the catalog.
3. As a Platen user, I want downloaded audio to keep its current directory layout, so that my existing media-server scan behavior continues to work.
4. As a Platen user, I want Jellyfin to scan Platen's music directory independently, so that Platen does not depend on Jellyfin's availability or catalog state.
5. As a user of another media server, I want the music directory to remain server-neutral, so that Platen does not require Jellyfin-specific naming or APIs.
6. As a Platen user, I want files and media-server records to have no effect on catalog membership, so that deleting or rescanning a media-server library cannot delete or create Albums in Platen.
7. As a Platen user, I want Platen to stop importing Jellyfin Albums, so that uncertain name-based matches cannot add the wrong Tidal Album.
8. As a Platen user, I want Platen to stop linking catalog Albums to Jellyfin records, so that server-local identifiers do not leak into the catalog.
9. As a Platen user, I want Platen to stop contacting MusicBrainz, so that the application has no unused metadata dependency after import is removed.
10. As an operator, I want Platen to start without Jellyfin configuration, so that I do not need to provide a server URL, API key, or user ID.
11. As an operator, I want Platen to start without any media server running, so that media-server outages cannot prevent startup or catalog work.
12. As an operator, I want retired import endpoints to return the router's normal not-found response, so that no dead compatibility API remains.
13. As an operator, I want no media-server rescan hook, so that Platen does not need server credentials or server-specific lifecycle logic.
14. As an operator, I want a clean initial database schema, so that new installations never store retired external identifiers.
15. As an operator, I accept recreating the database for this change, so that the implementation does not carry an upgrade path for a pre-release schema.
16. As a frontend developer, I want Album and Artist contracts to omit retired integration fields, so that the UI cannot accidentally depend on them.
17. As a backend developer, I want import concurrency state and summaries removed, so that no generic-looking machinery survives solely for a deleted feature.
18. As a backend developer, I want generated entities and TypeScript bindings to match the clean schema and DTOs, so that generated code does not preserve retired fields.
19. As a maintainer, I want active plans and current documentation to describe the new boundary, so that future work does not assume media-server reconciliation still exists.
20. As a maintainer, I want historical records to remain honest, so that old closed issues may explain the former Jellyfin integration without making it a current requirement.
21. As a maintainer, I want an architecture record for the one-way filesystem boundary, so that a future contributor does not reintroduce media-server state as an apparent convenience.
22. As a maintainer, I want the old import concurrency decision marked as superseded, so that readers do not mistake it for current architecture.
23. As a maintainer, I want stale issue dependencies and acceptance criteria corrected, so that remaining catalog work no longer waits for or tests a removed import feature.
24. As a maintainer, I want a search of live code and configuration to find no Jellyfin or MusicBrainz dependency, so that the removal is complete rather than cosmetic.

## Implementation Decisions

- Jellyfin is removed as an integration and as a current domain concept. Remove its HTTP client, response models, errors, route handlers, reconciliation logic, tests, module declarations, application state, and startup construction.
- Remove the import and import-status HTTP routes immediately. Do not add deprecation handlers, redirects, tombstone responses, or compatibility DTOs. Requests to the old paths receive the router's normal `404 Not Found` response.
- Remove Jellyfin configuration fields. Old configuration compatibility is not required.
- Remove import tracking in full, including the running guard, mutex-backed status, summaries, failures, and their tests. This machinery has no use outside the retired import.
- MusicBrainz is removed in full because the import is its only live caller. Remove its HTTP client, response models, request errors, module declaration, application state, and startup construction.
- The Artist schema loses its MusicBrainz identifier and associated index.
- The Album schema loses its Jellyfin identifier, MusicBrainz Release Group identifier, resolution method, and associated MusicBrainz index.
- Album and Artist API DTOs lose every retired identifier and the resolution method. All model-to-DTO conversions and constructors use the reduced shapes.
- Rewrite the initial SeaORM migration to create only the clean schema. Do not append a migration, preserve old column data, or support an existing database. A clean database is an explicit assumption.
- Regenerate SeaORM entities from the clean database through the repository script. Never edit generated entities by hand.
- Regenerate TypeScript bindings through the repository script after changing backend DTOs. Remove generated import DTO files that no longer have an exporting Rust type.
- Keep Tidal as the identity and metadata authority for Artists and Albums. Keep Antra as the download service.
- Albums enter the catalog only through an explicit Tidal-backed add action. Do not replace Jellyfin Import with filesystem scanning, tag scanning, directory inference, or another automatic discovery path.
- Preserve the current download placement and music-directory layout exactly. This work does not rename directories, retag files, change archive extraction, change skip-if-exists behavior, or add generic media-server normalization.
- Platen does not query, reconcile, identify, authenticate to, or notify any media server. A future rescan hook requires a separate decision and is not prepared by this work.
- Update the domain glossary to describe two external services, explicit Tidal-backed Album addition, and the one-way relationship between the music directory and media servers.
- Add an architecture record stating that media servers consume the music directory outside Platen's boundary. Record import, stored server identities, and scan requests as rejected approaches.
- Mark the single-concurrent-import architecture record as superseded. Update other current architecture records where they describe Jellyfin Import or MusicBrainz as a live creation path. Keep historically useful Jellyfin references in superseded ADRs and closed issues.
- The obsolete Jellyfin import issue was removed during planning because its requested feature is being deleted. Keep remaining active issues free of import blockers and acceptance criteria. Fresh-database verification covers explicit Album addition and download.
- The active multi-artist plan no longer prescribes import or MusicBrainz work. Historical closed issue records may retain references to the old behavior.
- Do not remove the HTTP dependency merely because the Jellyfin and MusicBrainz clients disappear. Tidal and Antra still use HTTP.

## Testing Decisions

- Prefer external behavior over private decision helpers. Deleted import decision functions and their unit tests should disappear rather than be rewritten around dead concepts.
- Use a fresh SQLite database as the main schema seam. Run the rewritten initial migration and assert that the Artist and Album tables omit all Jellyfin, MusicBrainz, and resolution-method columns and indexes while retaining the catalog and ordered-credit schema.
- Test the assembled HTTP router at the request boundary. Requests to both retired import paths must return `404 Not Found`. Prefer the existing router seam if one is available; otherwise extract the smallest router-construction seam needed to issue in-process requests without starting a server.
- Exercise the existing Album addition seam against a fresh database. Adding by Tidal ID still creates the Album, upserts every credited Artist, and stores ordered credits without any retired fields.
- Exercise the existing download seam or its established filesystem tests. Downloads must still land in the same Primary artist and Album directory used before this removal.
- Run backend tests after regenerating entities and bindings. Compilation is part of the check because model constructors across Album and Artist routes must adopt the reduced generated shapes.
- Run the TypeScript binding export test and verify that frontend DTOs omit retired fields and import DTO files no longer exist.
- Run frontend type checks, tests, and the production build. No frontend code should reference removed fields or routes.
- Start from a clean database for end-to-end verification. Existing database upgrade and rollback behavior are not test cases for this spec.
- Search live backend code, frontend code, configuration, the glossary, and active planning records for Jellyfin and MusicBrainz references. Any remaining live reference fails the removal. Explicit historical references in superseded ADRs and closed issues are allowed.
- Use the current catalog-derived directory tests as prior art for music-directory behavior. Use existing route and DTO export tests as prior art for API contracts. Add only the fresh-schema and retired-route seams needed to prove externally visible removal.

## Out of Scope

- Migrating, backing up, repairing, or preserving an existing SQLite database.
- Deleting catalog rows based on how they were originally discovered.
- Replacing Jellyfin Import with filesystem discovery, tag discovery, or another media-server integration.
- Triggering media-server scans after a download.
- Reading playback state, scan state, library membership, or metadata from any media server.
- Adding support for a different media server API.
- Changing the current music-directory hierarchy or file placement behavior.
- Normalizing embedded audio tags, artwork, codecs, or filenames for generic media-server compatibility.
- Reconsidering Tidal as the identity authority.
- Adding a new metadata authority to replace MusicBrainz.
- Providing compatibility responses for retired HTTP routes or configuration keys.

## Further Notes

The intended boundary is deliberately one-way: Platen writes catalog-derived
audio files, and outside software may read them. "Scannable by Jellyfin" is a
property of the existing file placement, not an integration contract.

This spec assumes a clean slate because the schema and domain changes are broad.
The initial migration is the only schema history that should remain after
related contract work is folded into it.
