# Platen

A personal music library manager. Owns a catalog of Artists and Albums and
reconciles it against four external services. The catalog is the only thing
platen owns; everything else is a source it reads from or writes to.

## Catalog entities

**Catalog**:
The set of Artists and Albums platen owns. The source of truth for what
platen knows about.
_Avoid_: library, collection

**Artist**:
A music artist represented in the catalog. Identified by its Tidal artist ID
and owning one or more Albums.
_Avoid_: musician, performer, act

**Album artist**:
The Artist an Album belongs to. An Album has exactly one, even when individual
tracks credit additional artists; track-level credits are metadata on the
tracks and never create catalog or library entities.
_Avoid_: album artist credits, main artist, featured artist

**Album**:
A release represented in the catalog. Identified by its Tidal album ID and
belonging to one Artist. A release may be an album, EP, or single. The name
"Album" is the entity's name, not a claim that every one is a full-length
album.
_Avoid_: release, record

## External services

**Tidal**:
A streaming service that acts as platen's identity authority, its
name-based search engine, and its source of album metadata. Tidal artist
and album IDs identify catalog Artists and Albums.

**MusicBrainz**:
A public music metadata authority. Platen uses two of its entities: the Release
Group (the canonical album across editions, pressings, and regions) and the
Artist Credit (the named party behind a release group).

**Jellyfin**:
A self-hosted media server hosting the user's actual music library. Platen does
not manage its audio files; it reconciles Jellyfin's album list against the
catalog and links Jellyfin albums to catalog Albums.

**Antra**:
A download service. Given a Tidal album URL, fetches lossless audio and hands
it to platen as a file. Download depends on the Album's Tidal ID being valid
in Tidal. See Library layout for what platen does with the file.

**Library layout**:
The directory structure of the user's music library, which platen owns and
derives from the catalog: one directory per Artist, containing one directory
per Album named for its title and release year. Structure provided by a
downloader is discarded.
_Avoid_: folder structure, music dir convention

## Reconciliation

**Import**:
The process of adding Jellyfin's album list to the catalog, or linking it
to existing Albums. Import is non-destructive: Artists and Albums absent
from Jellyfin remain in the catalog.
_Avoid_: sync, scan, ingest

**MusicBrainz Linking**:
Filling in an Artist's missing MusicBrainz identity when it is discovered
later, without changing the Artist's identity.
_Avoid_: matching, merging

**Jellyfin Linking**:
Attaching a Jellyfin album to an existing Album without creating a new one.
Sibling to MusicBrainz Linking, which is Artist-side.
_Avoid_: matching, sync
