# Spec: Place downloads in catalog-derived directories

## Problem Statement

When I download a collaboration album through platen, the download service
organizes the archive's folders using names that don't match the catalog
artist. My media server reads those folder names as artist names, so each
collaboration album spawns a phantom artist in my library (e.g. "BLCKK,
ISSBROKIE" appearing as its own artist next to BLCKK). My library fills with
junk artists that don't reflect the catalog I curate in platen.

## Solution

Platen stops trusting the archive's internal folder structure entirely. When a
download completes, platen extracts the archive to a temporary location,
locates the album directory inside it, and copies the files into a directory
it computes itself from catalog metadata: the Album's Artist name and the
Album's title and release year. The archive's own structure is discarded.
Singles already work this way; albums and EPs now follow the same convention,
so the whole library follows one layout rule that platen owns.

## User Stories

1. As a library owner, I want album and EP downloads placed in a directory
   named after the catalog Artist, so that my media server does not invent
   artists from archive folder names.
2. As a library owner, I want every download to follow one directory
   convention regardless of release type, so that my library is uniform.
3. As a library owner, I want the album directory to include the release
   year, so that albums with the same title don't collide.
4. As a library owner, I want the archive's internal folder names to be
   ignored, so that a downloader's tagging quirks never shape my library.
5. As a library owner, I want multi-disc albums to land in a single album
   directory, so that disc structure expressed in track filenames is
   preserved without extra folders.
6. As a library owner, I want a re-download to skip files that already
   exist in the destination, so that completed tracks are never lost.
7. As a library owner, I want temporary extraction artifacts cleaned up
   after a download, so that no disk space is wasted.
8. As a library owner, I want a download to fail with a clear error when the
   archive's contents don't match expectations, so that I never get silently
   misplaced files.
9. As a library owner, I want singles to keep their current placement
   behavior, so that nothing that works today regresses.
10. As a library owner, I want the destination directory created if it does
    not exist, so that a first download of a new artist or album succeeds.

## Implementation Decisions

- The destination directory for every download (single, EP, album) is
  computed in one place from catalog metadata:
  `{music_dir}/{artist.name}/{sanitized album title} ({release year})`.
  The artist name comes from the Album's catalog Artist, never from anything
  inside the archive.
- When `release_year` is 0 (date not yet refreshed), the current year is
  used as a fallback, matching existing single-download behavior.
- The archive is extracted to a temporary directory, not the destination.
  The system unzip tool is used; no new archive dependency is introduced.
- Antra's archives have a fixed shape: an artist folder containing an album
  folder containing the track files. Placement navigates that shape directly
  (exactly one directory at each of the two levels, files only at the
  bottom) and fails loudly on anything else. All files in the album
  directory are copied flat into the destination directory. No
  content-based detection, no per-disc handling.
- Existing files in the destination are skipped (skip-if-exists), matching
  current single-download semantics.
- Temporary extraction directories are removed after the copy completes,
  including on the failure path.
- The existing validation that the delivered file type matches the album
  type (FLAC for singles, ZIP for albums/EPs) is kept.
- Directory-name sanitization for catalog-derived names (the catalog title
  `Speakerboxxx/The Love Below` contains a path separator) is acknowledged
  as a real bug but is out of scope here; a handoff document will be
  written for it.

## Testing Decisions

- Tests target external behavior only: given an archive file and a
  destination directory, the placement step produces the expected directory
  contents and no archive-derived folder names appear anywhere under the
  destination.
- The seam is placement, not the download pipeline. Downloading splits into
  two pieces: transport (resolve, create job, poll, fetch bytes to a temp
  file; existing code, network-bound, unchanged and untested) and placement
  (given a downloaded archive and a destination directory, produce the right
  directory contents; pure filesystem-in, filesystem-out). Placement is
  where the new logic lives, so it is the seam.
- A test builds a fake archive in a temporary directory (using the zip tool
  already on the machine), calls placement with a destination, and asserts
  on the destination tree. No network, no mocking of the download service.
  Failure paths are tested the same way: an empty archive, an archive whose
  artist folder holds only files rather than another directory.
- The tested contract is the archive-shape assumption (artist folder, then
  album folder, then track files) plus the copy semantics (flat copy,
  skip-if-exists, temp cleanup on success and failure).
- The glue between transport and placement, the route handler computing the
  destination from catalog metadata, goes untested; exercising it would
  drag the database and network in for thin logic.
- Prior art: the release-date parsing tests in the album route module test
  pure logic at a module boundary; placement tests follow the same style,
  with the filesystem as the observable output.

## Out of Scope

- Retagging audio files; track tags are already correct.
- Retroactive cleanup of misfiled folders from past downloads; the owner
  will reorganize the existing library manually.
- Directory-name sanitization for filesystem-hostile characters in
  catalog-derived names; documented in a handoff document for a future
  session.
- Per-disc subfolder handling inside archives; the observed archive shape
  is flat, and a differently-shaped archive will fail loudly rather than
  be guessed at.
- Any change to how Jellyfin imports or links albums.

## Further Notes

- The archive shape was verified empirically with a multi-disc release
  (OutKast, Speakerboxxx/The Love Below): one flat directory of FLACs,
  disc numbers in filename prefixes (`1-01`, `2-21`), no cover art files.
- The decision to compute layout from catalog metadata is recorded in
  ADR 0003; the glossary now defines Library layout and Album artist, and
  the Antra entry no longer describes delivery mechanics.
