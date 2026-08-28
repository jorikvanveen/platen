# 0008: Contract: single source of truth for credits

**What to build:** The old single-artist model is gone. The album table loses
its artist column, the album DTO loses its single-artist field, and the
credits table is the only place album-artist links live. The artist-scoped
create and download routes and bare artist creation are removed. The initial
migration is rewritten to create the final schema directly, folding the later
incremental migrations into it; the database file is deleted and a fresh
database is assumed on next start (the app is not deployed; the sysadmin
deletes the DB before updating). Fresh-database setup is verified end to end.

Per the multi-artist credits plan (decisions 4, 9) and the expand-contract
sequence: this is the contract step, blocked by every migrate step.

**Blocked by:** 0005 (Import links all credited artists), 0006 (Read paths
expose credited artists), 0007 (Album-first frontend).

**Status:** ready-for-agent

- [ ] Album table has no artist column; credits table is the only album-artist link
- [ ] Album DTO has no single-artist field; frontend uses the credited-artists list
- [ ] Artist-scoped album create route, artist-scoped download route, and bare artist creation route removed
- [ ] Initial migration creates the final schema; later incremental migrations folded in and deleted
- [ ] Database file deleted; fresh database verified end to end (add album, import, download)
