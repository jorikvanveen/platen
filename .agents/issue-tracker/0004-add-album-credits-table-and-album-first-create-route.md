# 0004: Add album credits table and album-first create route

**What to build:** Adding an album by its Tidal ID credits every artist Tidal
lists on it. A new credits table links Albums to Artists with an explicit
position; the first credit is the Primary artist. A new album-first create
route takes just the Tidal album ID, fetches the album and its credited
artists, upserts the artists, inserts the album, and writes the ordered
credit rows. The existing artist-scoped create route keeps working unchanged,
so nothing breaks while this lands. The decision that Tidal's album-level
credits define the catalog's album-artist links is recorded as an ADR.

Per the multi-artist credits plan in `.agents/plan-multi-artist-credits.md`
and the glossary's Album credit and Primary artist definitions.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Credits table exists: album reference, artist reference, position; composite unique on album plus artist; no cascade deletes
- [ ] New create route: `POST /albums/{album_id}` adds the album and all its Tidal-credited artists in credit order
- [ ] Adding an already-existing album ID is idempotent (returns the existing album, no duplicate rows)
- [ ] Artists are upserted: inserted when absent, left untouched when present
- [ ] Old artist-scoped album create route still works
- [ ] ADR records "Tidal credits define album-artist links" and the trade-off against MusicBrainz credits
