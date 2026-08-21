# Artist search pagination

## Problem

The `/artist/add` page lets users search MusicBrainz for an artist and add them to
the local database. Today the page fetches
`${API_URL}/mb/search_artist/{query}`, which hits MusicBrainz's
`/artist?query={}&fmt=json` with no `limit`/`offset`. MusicBrainz defaults to
`limit=25, offset=0`, so the page can only ever show the first 25 results and
gives the user no way to see the rest. There is also a latent off-by-one in the
sibling paginated page (`artist/[artist_id]/add-release-group/[page]/+page.ts`),
whose `pages = Math.floor(count / 100)` is wrong when `count` is an exact
multiple of the page size.

## Destination

`/artist/add` becomes a single URL-driven page that paginates MusicBrainz artist
search results 100 at a time.

- **URL**: `/artist/add?q={query}&page={page}`. `q` defaults to `""`, `page`
  defaults to `0`. When `q === ""`, only the search form is rendered (no fetch,
  no results table). This keeps the existing "type a query first" UX.
- **Backend**: `GET /mb/search_artist/{query}?page={page}` returns
  `{ artist_count: usize, artists: Vec<ArtistSearchResult> }` (a new
  `ArtistSearchResponse`), mirroring the existing
  `ReleaseGroupResponse`/`get_release_groups` shape. The MusicBrainz call passes
  `limit=100&offset=page*100`. The `Pagination { page: usize }` extractor struct
  already present in `routes/mb.rs` is reused; no route registration change in
  `main.rs` is required (axum pulls `Query` from the URL string).
- **Frontend**: a new `+page.ts` load function reads `q` and `page` from
  `url.searchParams`, performs the fetch, and returns
  `{ query, page, pages, artists }`. The `+page.svelte` is rewritten to consume
  `data` instead of local state. Submitting the form navigates via `goto` to the
  same route with the new `q` and `page=0`, which re-fires `load`. Prev/Next
  controls below the table navigate to `page ∓ 1`; Prev is disabled when
  `page <= 0`, Next is disabled when `page >= pages - 1`. Per-row `addArtist`
  behavior (POST + redirect to `/artist/{id}`) is unchanged.
- **Page count math**: `pages = Math.ceil(count / 100)`. This is also applied as
  a one-line fix to the existing `add-release-group/[page]/+page.ts`, which
  currently uses `Math.floor` and is off-by-one when the total is a multiple of
  the page size. Adding pagination *controls* to the release-group page is out
  of scope; only the math is corrected there.

## Non-goals

- Adding pagination controls to `add-release-group/[page]/+page.svelte`.
- Changing the MusicBrainz client's rate-limiting or retry behavior.
- Changing `main.rs` route registration.
- Deep-linking the `q`/`page` params into SvelteKit's typed route params
  (they stay as `url.searchParams`, not path segments).
