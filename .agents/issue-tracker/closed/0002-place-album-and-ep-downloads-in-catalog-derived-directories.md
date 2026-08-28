# 0002: Place album and EP downloads in catalog-derived directories

**What to build:** Downloading any release type puts the files in
`music_dir/{artist name}/{album title} ({release year})/`. A collaboration
album lands under the catalog artist alone, so the media server stops
inventing phantom artists from track-level credits. The route computes the
destination for every release type from catalog metadata (a release year of
0 falls back to the current year; singles keep their existing move
behavior). Album and EP archives are extracted to a temporary directory, the
deepest directory's files are copied flat into the destination with
skip-if-exists semantics, and the temporary directory is cleaned up on
success and failure. An archive whose shape does not match expectations
fails loudly instead of guessing. Placement is tested at the filesystem
seam with fake archives built by the local zip tool: happy path,
skip-if-exists, and the failure shapes.

Per the spec and ADR 0003: the archive's internal structure is never
trusted, and the catalog is the source of truth for the library layout.

**Blocked by:** None (can start immediately).

**Status:** done

- [x] Album and EP downloads land in `music_dir/{artist name}/{album title} ({release year})/`
- [x] Singles still land in their existing directory layout
- [x] A release year of 0 falls back to the current year when naming the directory
- [x] Archive files are copied flat from the deepest directory; no archive-derived folder names appear under the destination
- [x] Existing files in the destination are skipped, not overwritten or deleted
- [x] The temporary extraction directory is removed on success and on failure
- [x] An empty archive, or one whose deepest level is a file, fails with a clear error
- [x] Placement tests cover the happy path, skip-if-exists, and failure shapes without network access
- [x] The leftover debug print in the job-status code path is removed
