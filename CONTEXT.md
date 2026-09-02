# Platen

A personal music library manager. Owns a catalog of Artists and Albums and
uses two external services to find metadata and download audio. Platen also
owns the layout of its music directory.

## Catalog entities

**Catalog**:
The set of Artists and Albums platen owns. The source of truth for what
platen knows about.
_Avoid_: library, collection

**Artist**:
A music artist represented in the catalog. Identified by its Tidal artist ID
and credited on one or more Albums. An Artist enters the catalog only as a
side effect of adding an Album it is credited on; it is never created on its
own, and it cannot be deleted while any Album credits it.
_Avoid_: musician, performer, act

**Album credit**:
An Artist credited on an Album. Every credited Artist becomes a catalog Artist
linked to the Album; credits come from Tidal's album-level artist list, and
track-level credits never create catalog entities. Credits are ordered; the
first is the Primary artist.
_Avoid_: album artist credits, featured artist

**Primary artist**:
The first credited Artist on an Album. Drives the Album's place in the library
layout and any display that needs "the" artist.
_Avoid_: main artist, album artist

**Album**:
A release represented in the catalog. Identified by its Tidal album ID and
credited to one or more Artists. An Album enters the catalog when the user adds
it from Tidal or a user-requested Music directory scan finds exactly one Tidal
match. A release may be an album, EP, or single. The name "Album" is the entity's
name, not a claim that every one is a full-length album.
_Avoid_: release, record

**Album cover**:
The optional image Tidal associates with an Album. An Album may remain in the
catalog without one.
_Avoid_: album picture, cover art

**Artist profile image**:
The optional image Tidal associates with an Artist. An Artist may remain in the
catalog without one.
_Avoid_: artist picture, avatar

**Downloaded Album**:
An Album whose audio is present in the Music directory, regardless of how it
arrived there. A Downloaded Album has an Album location; any other Album does not.
_Avoid_: Present Album, imported Album, installed Album

## External services

**Tidal**:
A streaming service that acts as platen's identity authority, its
name-based search engine, and its source of album metadata. Tidal artist
and album IDs identify catalog Artists and Albums.

**Antra**:
A download service. Given a Tidal album URL, fetches lossless audio and hands
it to platen as a file. Download depends on the Album's Tidal ID being valid
in Tidal. See Music directory for what platen does with the file.

**Music directory**:
The configured directory where platen stores and discovers audio. Its layout is
one directory per Primary artist, containing one directory per Album named for
its title and release year. Media servers may scan this directory independently.
_Avoid_: library, collection, folder structure, music dir convention

**Album location**:
An Album directory's path relative to the Music directory. Its presence means
Platen has observed audio for the Album; its absence means Platen has not.
_Avoid_: absolute path, album path, downloaded flag

**Music directory scan**:
A user-requested, read-only inspection of the Music directory using its Artist
and Album directory layout. Platen does not watch, move, rename, or delete files
during a scan.
_Avoid_: filesystem import, filesystem watcher, reconciliation

## Download workflow

**Download job**:
A user's request to fetch one Album through Antra and place its audio in the
Music directory. A job is queued, running, succeeded, failed, or cancelled; only
a queued job may be cancelled, and only one unfinished job may exist per Album.
_Avoid_: queue item, Antra job, download request
