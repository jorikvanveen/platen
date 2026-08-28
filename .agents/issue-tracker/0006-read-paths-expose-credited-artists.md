# 0006: Read paths expose credited artists

**What to build:** Reading the catalog shows every credited artist. The
per-artist albums endpoint returns albums the artist is credited on, each
album carrying its full ordered credit list (primary first). The album DTO
gains the credited artists while keeping the existing single-artist field
(meaning the Primary artist) so the current frontend keeps working. Download
moves to an album-scoped route and places files under the Primary artist's
directory, per ADR 0003; the artist-scoped download route keeps working until
the frontend migrates.

Per the multi-artist credits plan (decisions 6, 8, 10).

**Blocked by:** 0004 (Add album credits table and album-first create route).

**Status:** ready-for-agent

- [ ] Per-artist albums endpoint returns credited-on albums, ordered primary first within each album
- [ ] Album DTO carries the ordered credited artists; the old single-artist field still works and means the Primary artist
- [ ] Download route is album-scoped; destination directory comes from the Primary artist
- [ ] Old artist-scoped download route still works
- [ ] ts-rs exports regenerated for the changed DTOs
