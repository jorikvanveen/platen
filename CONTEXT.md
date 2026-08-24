# Platen

A personal music library manager. Owns a catalog of Artists and Albums and
reconciles it against four external services. The catalog is the only thing
platen owns; everything else is a source it reads from or writes to.

## Catalog entities

**Catalog**:
The set of Artist and Album rows platen owns in its own database. The source of
truth for what platen knows about.
_Avoid_: library, collection

**Artist**:
A music artist represented in the catalog. Identified by its Tidal artist ID
and owning one or more Albums.
_Avoid_: musician, performer, act

**Album**:
A music album represented in the catalog. Identified by its Tidal album ID and
belonging to one Artist.
_Avoid_: release, record

## External services

**Tidal**:
A streaming service that acts as platen's identity authority (Tidal artist and
album IDs are the catalog's primary keys), its name-based search engine, and the
source of album metadata for the Tidal-by-ID creation path.

**MusicBrainz**:
A public music metadata authority. Platen uses two of its entities: the Release
Group (the canonical album across editions, pressings, and regions) and the
Artist Credit (the named party behind a release group).

**Jellyfin**:
A self-hosted media server hosting the user's actual music library. Platen does
not manage its audio files; it reconciles Jellyfin's album list against the
catalog and records Jellyfin album IDs on Album rows as a link back.

**Antra**:
A download service. Given a Tidal album URL, fetches lossless audio and delivers
a zip platen extracts into the user's music directory. Download depends on the
Album's Tidal ID being valid in Tidal.

## Reconciliation

**Import**:
The process of scanning Jellyfin's album list and bringing the catalog into
agreement with it. Each Jellyfin album resolves to a created, linked, skipped, or
failed outcome.
_Avoid_: sync, scan, ingest

**MusicBrainz Linking**:
Filling in a missing MusicBrainz artist ID on an existing Artist when it is
discovered later, without changing the Artist's identity.
_Avoid_: matching, merging

**Jellyfin Linking**:
Recording a Jellyfin item's ID (and its MusicBrainz release group ID) on an
existing Album, without creating a new row. Sibling to MusicBrainz Linking,
which is Artist-side.
_Avoid_: matching, sync
