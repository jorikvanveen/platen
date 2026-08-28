# Album-first catalog with multi-artist credits

## Problem

The catalog assumes an Album has exactly one Artist. This is baked into the
schema (`album.artist_id` NOT NULL FK), the API (`POST /artists/{artist_id}/albums/{album_id}`),
the import path (keeps only Tidal's first album artist), and the frontend
(artist-first add flow). In reality releases are credited to multiple artists,
and Tidal already returns the full credit list (`Tidal::get_album_artists`);
the model just discards it.

## Decisions (settled in design session, 2026-08-28)

1. **Credits**: All Tidal album-level credited artists become catalog Artists
   linked to the Album. Track-level credits stay out of scope.
2. **Ordering**: Credits are ordered; position is stored; first = Primary
   artist.
3. **Album creation**: `POST /albums/{album_id}`. Fetches the album and its
   credited artists from Tidal, upserts every credited Artist, inserts the
   Album, links them all. Adding an artist first is no longer possible or
   required.
4. **Artist lifecycle**: `POST /artists/{id}` is removed. Artists come into
   existence only as a side effect of adding an Album they are credited on.
   An Artist with any credited Album cannot be deleted (no cascade).
5. **Library layout**: An Album lives under its Primary artist's directory
   only (recorded in ADR 0003). Other credited artists' pages show the album;
   the filesystem has one home for it.
6. **Download**: `POST /albums/{album_id}/download`; destination directory
   derived from the Primary artist.
7. **Import**: links all Tidal credited artists, same rule as the manual path.
   MB's first artist credit stays as the Tidal search string (known weak spot
   for collabs whose MB credit order differs from Tidal's).
8. **DTO**: `Album.artists: Artist[]` ordered by position; `artist_id` removed.
9. **Migration**: none. The initial table-creation migration is rewritten to
   the new schema; the later incremental migrations are folded into it and
   deleted. Existing `db.sqlite` is discarded (not deployed; sysadmin deletes
   the DB before updating).
10. **Read routes**: `GET /artists/{id}/albums` keeps its path, semantics
    become "albums this artist is credited on", primary-first ordering.
    `GET /artists` and `GET /artists/{id}` unchanged.

## Work breakdown

### Backend: schema and entities

- Rewrite `migration/src/m20220101_000001_create_table.rs`:
  - `album` loses `artist_id`.
  - New `album_artist` table: `album_id` (FK), `artist_id` (FK), `position`
    (int), PK on `(album_id, artist_id)`. No `ON DELETE CASCADE` on either FK.
  - Fold in `m20260824_180325_rename_musicbrainz_id_columns` and
    `m20260825_000001_add_album_release_date` so the initial migration creates
    the final schema directly; delete those files and update
    `migration/src/lib.rs`.
- Regenerate entities (`sea-orm-cli generate entity` per existing codegen
  provenance headers) or hand-edit `entity/album.rs`:
  - Drop `artist_id` column and `Artist` relation.
  - Add `AlbumArtist` entity with relations to both `album` and `artist`.
- Delete `db.sqlite`.

### Backend: routes

- `routes/album.rs`:
  - New `create`: `POST /albums/{album_id}`. Tidal `get_album` +
    `get_album_artists`; upsert each credited artist (insert if absent, no
    update if present); insert album; insert `album_artist` rows with
    position. Idempotent on album ID like today.
  - `fetch_all` becomes: find artist, join through `album_artist`, order by
    position within each album (primary first), return `dto::Album` with
    embedded artists.
  - `download` moves to `POST /albums/{album_id}/download`; destination from
    the Primary artist (position 0).
  - `dto::Album`: replace `artist_id: String` with `artists: Vec<dto::Artist>`
    (ordered). Regenerate ts-rs exports.
- `routes/artist.rs`:
  - Remove `create` and its route registration.
  - `get` and `list` unchanged.
- `main.rs`: update route table (`POST /albums/{album_id}`,
  `POST /albums/{album_id}/download`, remove artist create, remove
  artist-scoped album create).

### Backend: import

- `routes/jellyfin.rs::resolve_album`:
  - After the Tidal hit, fetch all credited artists (`get_album_artists`),
    upsert each, and insert `album_artist` rows with position.
  - Keep MB first-credit search string as is.
  - Update `decide_album_insert` and its tests for the new shape.

### Frontend

- Regenerated DTOs land via ts-rs (`Album.artists`, no `artist_id`).
- New global "Add album" page (`/album/add`): search Tidal albums by name
  (needs a new backend route `GET /tidal/search/albums` wrapping
  `Tidal::find_album`), show credited artists per hit, one click adds via
  `POST /albums/{album_id}`.
- `artist/add` page becomes search-and-add-releases: search Tidal artists,
  pick one, browse their Tidal discography (existing
  `GET /tidal/artists/{id}`), add albums from there. No bare artist creation.
- Per-artist page (`/artist/[artist_id]`): keep discography browse; album
  rows show all credited artists joined "A, B & C", primary first.
- Homepage: unchanged (artist list), per Q17.

### Docs

- `CONTEXT.md` already updated (Album credit, Primary artist, Artist, Album).
- ADR 0003 already amended (primary-artist-only directory).
- Candidate new ADR: "Tidal credits define album-artist links" (hard to
  reverse once data accumulates, surprising without context, real trade-off
  vs MB credits). Write it when implementing.

## Out of scope

- Track-level credits.
- Refreshing credits of albums added before the change (fresh start makes it
  moot).
- Search-string tuning for collab albums (Q14 known weak spot).
- PR splitting (decided after the plan is reviewed).
