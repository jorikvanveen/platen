# Media servers are outside Platen's boundary

Status: accepted

Platen owns its Catalog and the layout of its Music directory. Media servers
may scan the Music directory, but platen does not query their catalogs,
reconcile their state, store their identifiers, or request scans. Albums enter
the Catalog only through an explicit Tidal-backed add action. Importing a media
server's state made an external index act like a second catalog, while storing
server-local identifiers tied catalog rows to one installation.

## Considered Options

- Import media-server records into the Catalog. Rejected because media-server
  state does not represent the user's intent for Platen's Catalog.
- Store media-server identifiers without importing. Rejected because the
  identifiers have no use unless Platen resumes reading server state.
- Request a media-server scan after writing files. Rejected for now because it
  requires server-specific configuration and credentials. A future scan hook
  needs a separate decision.

## Consequences

- Media-server availability cannot block Platen startup, catalog work, or
  downloads.
- Platen keeps its existing catalog-derived Music directory layout so Jellyfin
  and other media servers can scan the files independently.
- Files and media-server records never create or delete catalog Albums.
- This decision supersedes ADR-0002.
