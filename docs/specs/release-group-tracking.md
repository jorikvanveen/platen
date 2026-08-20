# Release Group Tracking and Download

## Status

Current behavior as of the release/release-group naming cleanup (2026-08-20).

## Background

Platen integrates with two external services:

- **MusicBrainz** — read-only metadata source for artists and release groups.
- **Tidal** (via the Antra downloader) — the source of audio for downloads.

MusicBrainz distinguishes two concepts:

- A **release group** is an abstract album (e.g. "OK Computer"). It has a title, primary type (album/single/EP/…), and first release date, but no concrete tracklist or format.
- A **release** is a specific edition of a release group (a particular CD pressing, vinyl, digital release, …) with barcode, country, media, tracks, labels, etc.

Platen operates at the **release-group** level. The user tracks release groups per artist and downloads them; Platen searches Tidal for the album by `artist + title` and downloads the first match. Specific editions are not modeled.

A "fetch a specific release from a release group" feature may be added in the future. It is out of scope for now and is not modeled in the schema or the API.

## Scope

### In scope

- Listing an artist's tracked release groups.
- Adding a MusicBrainz release group to an artist's tracked list.
- Downloading a tracked release group's audio via Tidal.
- Marking a tracked release group as downloaded after a successful download.
- Browsing MusicBrainz release groups for an artist (paginated) to find ones to add.

### Out of scope

- Specific editions (MusicBrainz releases). The `Release` concept is not persisted or exposed.
- Tracklists, barcodes, formats, labels, cover art.
- Downloading from sources other than Tidal (a `Youtube` downloader exists but its `Downloader` impl is commented out and not wired into the router).
- Re-downloading, deletion, or editing of tracked release groups.

## Data model

Two tables, both using the MusicBrainz MBID as the primary key.

### `artist`

| Column | Type | Notes |
|---|---|---|
| `musicbrainz_id` | `text` | Primary key. MusicBrainz artist MBID. |
| `name` | `text` | Artist name, copied from MusicBrainz at creation time. |

### `release_group`

| Column | Type | Notes |
|---|---|---|
| `musicbrainz_id` | `text` | Primary key. MusicBrainz release-group MBID. |
| `title` | `text` | Release-group title, copied from MusicBrainz at creation time. |
| `artist_id` | `text` | FK → `artist.musicbrainz_id`, `ON DELETE CASCADE`, `ON UPDATE CASCADE`. Constraint name: `artist-release-group-fk`. |
| `type` | `text` | Primary type from MusicBrainz (e.g. `Album`, `Single`, `EP`). |
| `downloaded` | `boolean` | Whether the release group has been successfully downloaded. |

The foreign-key constraint was originally named `artist-release-fk`; migration `m20260820_170510_rename_release_group_fk` renames it to `artist-release-group-fk` (SQLite cannot rename a constraint in place, so the migration recreates the table, preserving data).

## API

All routes are served by the `platen-backend` (Axum). The frontend calls these; they are not a public API.

### Tracked release groups

| Method | Path | Handler | Description |
|---|---|---|---|
| `GET` | `/artist/{artist_id}/release-groups` | `routes::release_group::fetch_all` | List all release groups tracked for the artist. Returns `Json<Vec<release_group::Model>>`. |
| `POST` | `/artist/{artist_id}/release-group/{release_group_id}` | `routes::release_group::create` | Register a MusicBrainz release group as tracked for the artist. Fetches the release group from MusicBrainz, inserts a row, returns the created `Json<release_group::Model>`. Idempotency is not enforced; inserting an already-tracked MBID will fail on the primary key. |
| `POST` | `/artist/{artist_id}/release-group/{release_group_id}/download` | `routes::release_group::download` | Download the release group's audio via Tidal. Looks up the tracked row, calls `Antra::download_release_group(artist_name, title, music_dir)`, and on success marks `downloaded = true`. |

### MusicBrainz proxy

| Method | Path | Handler | Description |
|---|---|---|---|
| `GET` | `/mb/artist/{artist_id}` | `routes::mb::get_artist` | Fetch an artist from MusicBrainz. |
| `GET` | `/mb/search_artist/{query}` | `routes::mb::search_artist` | Search MusicBrainz for artists. |
| `GET` | `/mb/artist/{artist_id}/release-groups?page={page}` | `routes::mb::get_artist_release_groups` | Paginated list of an artist's release groups from MusicBrainz (100 per page). Used by the "add release group" flow to browse what's available to track. |

### Artist management

| Method | Path | Handler | Description |
|---|---|---|---|
| `GET` | `/artist` | `routes::artist::list` | List tracked artists. |
| `GET` | `/artist/{id}` | `routes::artist::get` | Get a tracked artist. |
| `POST` | `/artist/{id}` | `routes::artist::create` | Register an artist from MusicBrainz. |

## Download flow

1. The frontend POSTs to `/artist/{artist_id}/release-group/{release_group_id}/download`.
2. The backend resolves the tracked `release_group` row to get the title, and the `artist` row to get the name.
3. `Antra::download_release_group(artist_name, title, music_dir)`:
   1. Searches Tidal for `{artist_name} {title}`.
   2. Takes the first album result.
   3. Resolves the album URL, creates a download job, polls job status until complete, downloads the resulting zip, and unzips it into `music_dir`.
4. On success, the backend sets `release_group.downloaded = true`.

The download is synchronous from the HTTP client's perspective: the request does not return until the download and unzip complete (or fail). The frontend tracks per-id `"downloading"` / `"done"` state in memory; there is no server-side job queue or progress reporting.

## Frontend

SvelteKit app. Two routes per artist:

- `/artist/{artist_id}` — lists the artist's tracked release groups with a per-row Download button and an "Add release group" link. Download state is held in a local `$state` map keyed by `musicbrainz_id`.
- `/artist/{artist_id}/add-release-group/{page}` — paginated browse of the artist's MusicBrainz release groups (100 per page) with a per-row Add button. Added ids are tracked in a local `$state` array to hide already-added rows in the current session.

Types:

- `ReleaseGroup` (artist page) — `{ musicbrainz_id, title, artist_id, downloaded }`, mirrors the backend `release_group::Model`.
- `ReleaseGroup` (add-release-group page) — `{ primary_type, disambiguation, id, first_release_date, title }`, mirrors the MusicBrainz release-group shape.
- `ReleaseGroupResp` — `{ release_group_count, release_groups: ReleaseGroup[] }`, the MusicBrainz paginated response.

## Open issues / known limitations

- **No download progress or job tracking.** A download request blocks until completion; failures surface only as a failed response. Long downloads hold an HTTP connection open.
- **No idempotent add.** Re-adding a tracked release group fails on the primary-key constraint rather than returning the existing row.
- **No deletion.** Tracked release groups cannot be removed.
- **First Tidal match wins.** Download picks the first Tidal search result for `{artist} {title}`; there is no disambiguation or verification that it's the right album.
- **`Youtube` downloader is dead code.** Its `Downloader` impl is commented out and it is not wired into the router.
- **`ts-rs` dependency is unused.** Frontend types are hand-written; the backend has no `#[derive(TS)]` attributes.
- **Stale commented-out block in `main.rs`** references the removed `Release` struct and the old `download_release` method signature. It does not affect compilation.