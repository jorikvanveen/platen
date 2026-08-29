# 0006: Read paths expose credited artists

**What to build:** Reading the catalog shows every credited artist. The
per-artist albums endpoint returns albums the artist is credited on, each
album carrying its full ordered credit list (primary first). The album DTO
exposes that list and drops the old single-artist field. The frontend reads
the full credit list. Download moves to an album-scoped route and places files
under the Primary artist's
directory, per ADR 0003. The artist-scoped download route remains temporarily
for compatibility.

Per the multi-artist credits plan (decisions 6, 8, 10).

**Blocked by:** 0004 (Add album credits table and album-first create route).

**Status:** ready-for-agent

- [ ] Per-artist albums endpoint returns credited-on albums, ordered primary first within each album
- [ ] Album DTO carries the ordered credited artists and has no single-artist field
- [ ] Download route is album-scoped; destination directory comes from the Primary artist
- [ ] Old artist-scoped download route still works
- [ ] ts-rs exports regenerated for the changed DTOs
