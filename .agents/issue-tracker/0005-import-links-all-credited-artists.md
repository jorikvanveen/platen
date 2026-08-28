# 0005: Import links all credited artists

**What to build:** A Jellyfin import credits every artist Tidal lists on an
album, not just the first one. When import resolves an album to a Tidal hit,
it fetches the full credit list, upserts each credited artist, and writes
ordered credit rows. The MusicBrainz first-credit search string stays as is;
its collab-ordering weak spot is a known, accepted limitation. The import
decision logic and its tests move to the credits model.

Per the multi-artist credits plan (decision 7) and the glossary's Album
credit definition.

**Blocked by:** 0004 (Add album credits table and album-first create route).

**Status:** ready-for-agent

- [ ] Import upserts every Tidal-credited artist on a resolved album and writes credit rows with position
- [ ] Manual path and import path follow the same credit rule
- [ ] MusicBrainz first-credit search string unchanged
- [ ] Import decision logic updated to the credits model; no reference to a single album artist remains in the import path
- [ ] Import tests cover a multi-artist album end to end
