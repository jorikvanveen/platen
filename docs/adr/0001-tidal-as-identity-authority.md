# 0001. Tidal as identity authority

Date: 2026-08-24

## Context

Platen's catalog has two tables, Artist and Album, and both are discovered
through three different doors:

1. The user picks something in Tidal and adds it by Tidal ID.
2. Import scans Jellyfin, which exposes MusicBrainz release group and artist
   IDs as Provider IDs.
3. MusicBrainz is queried for metadata to drive a Tidal name search.

Each of Jellyfin, MusicBrainz, and Tidal issues its own stable IDs. The catalog
needs one primary key per row, so a choice had to be made: which service's ID
is the row's identity?

The alternatives were:

- **MusicBrainz release group ID (album) / artist ID (artist).** MusicBrainz is
  the most authoritative and stable, and Jellyfin already exposes it. But not
  every catalog row has a MusicBrainz ID: the Tidal-by-ID creation path and the
  name_search path can produce rows before MusicBrainz is consulted, and
  MusicBrainz lookups can fail. Making MBID the PK would block row creation on
  an MBID being present.
- **Jellyfin ID.** Jellyfin is the library platen ultimately serves, so its IDs
  are always available during Import. But rows can be created independently of
  Jellyfin (the Tidal-by-ID path), and Jellyfin IDs are only meaningful to the
  user's specific server. An album not yet in Jellyfin would have no key.
- **Tidal ID.** Tidal is the one service every creation path touches: the
  Tidal-by-ID path takes it directly, and the Import path always ends in a
  Tidal search to get a Tidal album ID before inserting. Download also depends
  on a valid Tidal ID (Antra is given a Tidal album URL).

## Decision

Tidal artist and album IDs are the primary keys of Artist and Album. The other
services' IDs (MusicBrainz release group, Jellyfin album) are stored as
optional cross-reference columns on the row.

## Consequences

- A catalog row cannot exist without a Tidal ID. Albums present in Jellyfin but
  absent from Tidal (no search hit) fail Import rather than getting a row.
- `name_search` rows carry a Tidal ID chosen by taking the first Tidal search
  hit with no verification. The row's identity is therefore a best guess in
  that case; a wrong guess means Download fetches the wrong album, and a later
  Tidal-by-ID add of the correct album creates a second row instead of linking.
- The MusicBrainz cross-reference columns are named after the entity they
  hold: `musicbrainz_release_group_id` on Album, `musicbrainz_artist_id` on
  Artist. An earlier schema used one `musicbrainz_id` column on both tables,
  which made the entity type ambiguous; migration
  `m20260824_180325_rename_musicbrainz_id_columns` renamed them.
- Jellyfin Linking and MusicBrainz Linking are first-class operations because
  the row's identity is fixed at creation but its cross-references arrive
  later.
- Switching identity authorities later would require re-keying both tables, the
  foreign key, and every DTO. Costly.
