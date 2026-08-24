# Platen domain glossary

Platen is a personal music library manager. It keeps a small catalog of Artists
and Albums and reconciles it against four external services. The catalog is the
only thing platen owns; everything else is a source it reads from or writes to.

## Catalog

The set of Artist and Album rows platen owns in its own database. The catalog is
the source of truth for "what platen knows about." Everything below either feeds
into it or reads out of it.

## Artist

A music artist, as a row in the catalog. Identified by its Tidal artist ID. Has a
name and an optional MusicBrainz artist ID. An Artist owns many Albums.

## Album

A music album, as a row in the catalog. Identified by its Tidal album ID.
Belongs to one Artist. Carries a title, an album type, and two optional
cross-references: a Jellyfin ID and a MusicBrainz release group ID.

The Album's primary identifier is a Tidal album ID, even though the row is often
discovered through Jellyfin or MusicBrainz first. See
`docs/adr/0001-tidal-as-identity-authority.md`.

An Album's Tidal ID is either authoritative (someone picked the album in Tidal)
or inferred from a name search with no verification. The catalog records which,
since an inferred ID is a best guess and a wrong one means Download fetches the
wrong album.

## External services

### Tidal

A streaming service. Acts as platen's identity authority: Tidal artist and
album IDs are the primary keys of the catalog. Also the search engine used when
platen has a name but no Tidal ID, and the source of album metadata (title,
type) for rows created through the Tidal-by-ID path.

### MusicBrainz

A public music metadata authority. Platen uses two of its entities:

- Release Group — the canonical "album" across editions, pressings, and
  regions. A MusicBrainz release group ID is the stable cross-service album
  identifier stored on an Album as its `musicbrainz_release_group_id`.
- Artist Credit — the named party behind a release group. Platen reads the
  first credit's name and ID to drive Tidal name search and to backfill an
  Artist's `musicbrainz_artist_id`.

The two columns used to share one name (`musicbrainz_id`), which made it
ambiguous which MusicBrainz entity a value referred to. They are now named
after the entity they hold: `musicbrainz_release_group_id` on Album,
`musicbrainz_artist_id` on Artist. See migration
`m20260824_180325_rename_musicbrainz_id_columns` and
`docs/adr/0001-tidal-as-identity-authority.md`.

### Jellyfin

A self-hosted media server hosting the user's actual music library. Platen does
not manage the audio files in Jellyfin; it reconciles Jellyfin's album list
against the catalog (see Import) and stores Jellyfin album IDs on Album rows as
a link back to the library.

Jellyfin albums carry Provider IDs, a map from provider name to that provider's
ID. Platen reads `MusicBrainzReleaseGroup` and `MusicBrainzArtist` from this map
to drive reconciliation.

### Antra

A download service. Given a Tidal album URL, fetches lossless audio and
delivers a zip platen extracts into the user's music directory. Download depends
on an Album's Tidal ID being valid in Tidal.

## Reconciliation

### Import

The process of scanning Jellyfin's album list and bringing the catalog into
agreement with it. Each Jellyfin album is resolved to an Album row, with four
possible outcomes tracked in an Import Summary:

- Created — a new Album row was inserted, keyed by a Tidal ID.
- Linked — an existing Album row had its Jellyfin ID (and/or MusicBrainz ID)
  filled in.
- Skipped — the Album row was already linked to this Jellyfin album, so nothing
  happened.
- Failed — the album could not be resolved (e.g. no MusicBrainz release group
  ID on the Jellyfin item, MusicBrainz or Tidal lookup failed, no Tidal search
  hits). Recorded as an Import Failure with a reason.

Import only processes Jellyfin items that carry a MusicBrainz release group ID.
Items without one are skipped without error.

### Backfill

Filling in a missing MusicBrainz ID on an existing catalog row when it is
discovered later, without changing the row's identity. An Artist created from a
Tidal-only flow can later have its MusicBrainz artist ID backfilled during
Import.

### Linking

Connecting an existing catalog row to another service by recording that
service's ID on the row, without creating a new row. Distinct from Creating,
which inserts a new row keyed by a Tidal ID.
