# Group downloaders and musicbrainz under services

## Context

The backend has two flat sibling modules, `src/downloaders/` and `src/musicbrainz/`, that both hold external-service clients. `downloaders` defines a `Downloader` trait, a `RateLimit` helper, and the `Antra`/Tidal client. `musicbrainz` defines the `Musicbrainz` client, `RequestError`, and DTOs. `musicbrainz` imports `RateLimit` from `downloaders`.

We want them grouped under a new `services` module so the directory layout reflects that these are the same kind of thing.

## Decision

Move both modules under a new `src/services/`. Callers use the full path `crate::services::downloaders::...` and `crate::services::musicbrainz::...`. No re-exports.

Plain `mod` with two children. No prelude, no abstraction. New services slot in later as another `pub mod` under `services`.

## Out of scope

- `RateLimit` stays in `downloaders` even though `musicbrainz` uses it. Promoting it to `services::shared` is a separate refactor.
- `musicbrainz/release_group.rs` keeps its self-referential `use crate::musicbrainz::Musicbrainz;` import, updated to the new crate path. Switching it to `super::Musicbrainz` is a separate cleanup.
- `Config` in `src/main.rs` stays private. Making it `pub(crate)` explicitly is a separate change.

## Edit plan

### Filesystem moves

- `src/downloaders/` to `src/services/downloaders/`
- `src/musicbrainz/` to `src/services/musicbrainz/`

### New file

`src/services/mod.rs`:

```rust
pub mod downloaders;
pub mod musicbrainz;
```

### `src/main.rs`

Replace `mod downloaders;` and `mod musicbrainz;` with `mod services;`. Update the use block at L16-19 to `crate::services::{downloaders::{Downloader, antra::Antra}, musicbrainz::Musicbrainz}`.

### Internal cross-references

- `services/downloaders/mod.rs:5` updates `crate::musicbrainz::release_group::ReleaseGroup` to `crate::services::musicbrainz::release_group::ReleaseGroup`.
- `services/downloaders/antra.rs` use block updates `crate::downloaders::{...}` to `crate::services::downloaders::{...}`. `crate::Config` stays.
- `services/musicbrainz/mod.rs:6` updates `crate::downloaders::RateLimit` to `crate::services::downloaders::RateLimit`.
- `services/musicbrainz/release_group.rs:4` updates `crate::musicbrainz::Musicbrainz` to `crate::services::musicbrainz::Musicbrainz`.

### Route call sites

- `routes/artist.rs:6` updates the use block to `crate::services::musicbrainz::RequestError`.
- `routes/mb.rs:9-15` updates the use block to `crate::services::musicbrainz::{self, artist::{ArtistSearchResponse, ReleaseGroupResponse}}`. L21 and L28 use the `self` alias and resolve without further edits.
- `routes/release_group.rs` updates L15, L28, L32, L35 to `crate::services::...` paths. L15 is `crate::downloaders::Downloader`. L28, L32, L35 are `crate::musicbrainz::RequestError::...`.

### Validation

Run `cargo check` in `platen-backend`. Confirm it compiles.
