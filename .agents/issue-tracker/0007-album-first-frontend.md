# 0007: Album-first frontend

**What to build:** Adding music starts from the album, never from the artist.
A global add-album page searches Tidal albums by name, shows each hit's
credited artists, and adds the whole bundle in one click. The artist search
page becomes search-and-add-releases: picking an artist browses their Tidal
discography, and adding a release creates the artist as a side effect. Bare
artist creation is gone. The per-artist page shows every credited artist on
each album row, primary first, joined "A, B & C". The homepage artist list is
unchanged.

Per the multi-artist credits plan (decisions 3, 4, 17, 18, 19).

**Blocked by:** 0004 (Add album credits table and album-first create route),
0006 (Read paths expose credited artists).

**Status:** ready-for-agent

- [ ] Global add-album page: Tidal album search, credited artists shown per hit, one click adds
- [ ] Album search backend route exists, wrapping the existing Tidal album search service
- [ ] Artist search page adds releases instead of creating a bare artist; adding a release creates the artist
- [ ] Per-artist page shows all credited artists per album, primary first
- [ ] Homepage unchanged
