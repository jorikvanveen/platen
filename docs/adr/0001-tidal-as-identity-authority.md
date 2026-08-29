# Tidal as identity authority

Status: accepted

ADR-0005 supersedes the parts of this record that treat Jellyfin Import,
MusicBrainz linking, or external cross-reference identifiers as current
behavior.

Platen's catalog needs one primary key per Artist and Album row, but rows are
discovered through three creation paths (Tidal-by-ID, Jellyfin Import,
MusicBrainz-driven name search) that touch different services. Tidal artist and
album IDs are the primary keys, because Tidal is the only service every creation
path touches; the other services' IDs are stored as optional cross-reference
columns.

## Considered Options

- **MusicBrainz release group / artist ID.** Most authoritative and stable, and
  already exposed by Jellyfin. But not every row has one at creation time: the
  Tidal-by-ID and name_search paths can produce rows before MusicBrainz is
  consulted, and lookups can fail. Making MBID the primary key would block row
  creation on an MBID being present.
- **Jellyfin ID.** Always available during Import, but rows can be created
  independently of Jellyfin (the Tidal-by-ID path) and Jellyfin IDs are only
  meaningful to the user's specific server. An album not yet in Jellyfin would
  have no key.
- **Tidal ID.** The one service every creation path touches: the Tidal-by-ID
  path takes it directly, and Import always ends in a Tidal search to get a
  Tidal album ID before inserting. Download also depends on a valid Tidal ID.

## Consequences

- A catalog row cannot exist without a Tidal ID. Albums present in Jellyfin but
  absent from Tidal (no search hit) fail Import rather than getting a row.
- `name_search` rows carry a Tidal ID chosen by taking the first Tidal search
  hit with no verification. The row's identity is a best guess in that case; a
  wrong guess means Download fetches the wrong album, and a later Tidal-by-ID
  add of the correct album creates a second row instead of linking.
- Jellyfin Linking and MusicBrainz Linking are first-class operations because
  the row's identity is fixed at creation but its cross-references arrive
  later.
- Switching identity authorities later would require re-keying both tables, the
  foreign key, and every DTO. Costly.
