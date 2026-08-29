# Tidal album credits define album-artist links

An album can be credited to several artists, and platen needs one answer to
"which artists is this album linked to". MusicBrainz and Tidal disagree on
that list often enough that picking a source is a real decision: MusicBrainz
has richer credit data but its release-group artist credits don't always
match what Tidal shows on the album, and Tidal is already platen's identity
authority (ADR 0001). We decided that Tidal's album-level artist list, in its
order, defines the catalog's album-artist links; the first credit is the
Primary artist. MusicBrainz credits are still used for search strings and
artist identity linking, but never to add, remove, or reorder an album's
artist links.

## Consequences

- Adding an album by Tidal ID credits every artist Tidal lists, in Tidal's
  order. A collaboration whose MusicBrainz credit order differs will show
  Tidal's order in platen.
- The Jellyfin import keeps the MusicBrainz first-credit artist as the Tidal
  search string, so an album whose MB credit order differs from Tidal's may
  resolve to a different edition than expected. Known weak spot, accepted.
- Once albums accumulate, switching to MusicBrainz credits would mean
  rewriting every album-artist link and re-deciding primary artists. This is
  expensive to reverse, which is why it is recorded here.
- Track-level credits stay out of scope; only Tidal's album-level list
  creates links.
