# Media servers are outside Platen's boundary

Status: accepted

ADR-0007 supersedes the parts of this record that prohibit Music directory
scans from creating Catalog Albums or updating their downloaded state.

Platen owns its Catalog and the layout of its Music directory. Media servers
may scan the Music directory, but Platen does not query their catalogs,
reconcile their state, store their identifiers, or request scans. Albums enter
the Catalog only through an explicit Tidal-backed add action. Tidal supplies
Artist and Album identity, plus ordered Artist credits. Platen does not consult MusicBrainz
or store MusicBrainz identifiers. Importing a media server's state made an
external index act like a second catalog, while storing external identifiers
tied catalog rows to services that do not own Platen's Catalog.

## Considered Options

- Import media-server records into the Catalog. Rejected because media-server
  state does not represent the user's intent for Platen's Catalog.
- Store media-server identifiers without importing. Rejected because the
  identifiers have no use unless Platen resumes reading server state.
- Keep MusicBrainz as a second metadata authority. Rejected because Jellyfin
  Import was its only live caller, and explicit Tidal-backed addition already
  supplies Album identity and Artist credits.
- Request a media-server scan after writing files. Rejected for now because it
  requires server-specific configuration and credentials. A future scan hook
  needs a separate decision.

## Consequences

- Media-server availability cannot block Platen startup, catalog work, or
  downloads.
- Platen keeps its existing catalog-derived Music directory layout so Jellyfin
  and other media servers can scan the files independently.
- Media-server records never create or delete catalog Albums. ADR-0007 permits
  a Music directory scan to create an Album only after a unique Tidal match.
- Platen stores no Jellyfin or MusicBrainz cross-reference identifiers.
- ADR-0001 and ADR-0004 remain accepted, but this decision supersedes their
  descriptions of Jellyfin Import, MusicBrainz linking, and external
  cross-reference identifiers as current behavior.
- ADR-0003 remains accepted. Its Jellyfin references are historical context;
  media servers now consume the catalog-derived Music directory from outside
  Platen's boundary.
- This decision supersedes ADR-0002.
