# Music directory scans can create Catalog Albums

Status: accepted

Platen may create an Album when a user-requested Music directory scan finds exactly one high-confidence Tidal match for an Album directory. The filesystem is a discovery source and records where audio is present, while Tidal remains the identity, metadata, and Album credit authority. This reverses the part of ADR-0005 that prohibited filesystem-derived Catalog creation without bringing media servers back inside Platen's boundary.

## Consequences

- An Album's optional location relative to the Music directory is the sole record of whether its audio is downloaded, regardless of how the files arrived.
- A scan clears locations Platen cannot observe but never deletes Catalog Albums or modifies files.
- Ambiguous, unmatched, incomplete, and duplicate Tidal matches do not create or change Catalog Albums.
- Media-server records remain outside Platen's boundary and cannot create Catalog identity.
