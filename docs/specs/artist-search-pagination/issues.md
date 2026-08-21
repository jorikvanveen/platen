# Artist search pagination — implementation plan

One issue. The feature is a single vertical slice: "paginated artist search on
`/artist/add`", which touches the MusicBrainz client, the axum route handler, and
the SvelteKit route together. A one-line correctness fix to the sibling
`add-release-group` page's pagination math is bundled in (it's part of the same
spec and has no independent value).

No dependencies; no parallelization to schedule.

---

## Issue — Paginated artist search on `/artist/add`

**Scope** (files):
- `platen-backend/src/musicbrainz/artist.rs`
- `platen-backend/src/routes/mb.rs`
- `platen-frontend/src/routes/artist/add/+page.ts` (NEW)
- `platen-frontend/src/routes/artist/add/+page.svelte` (REWRITE)
- `platen-frontend/src/routes/artist/[artist_id]/add-release-group/[page]/+page.ts` (one-line fix)

**What changes**

### Backend — `musicbrainz/artist.rs`

1. `Musicbrainz::search_artist` signature changes from
   `search_artist(&self, query: &str) -> Result<Vec<ArtistSearchResult>, RequestError>`
   to
   `search_artist(&self, query: &str, page: usize) -> Result<ArtistSearchResponse, RequestError>`.
2. The MusicBrainz URL becomes
   `{BASE_URL}/artist?query={}&limit=100&offset={page * 100}&fmt=json`
   (mirror `get_release_groups` in the same file).
3. `ArtistSearchResponse` becomes a public struct with `ts_rs::TS` so the
   frontend gets a typed binding (mirroring `ArtistSearchResult`'s existing
   convention):
   ```rust
   #[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
   #[ts(export)]
   #[serde(rename_all(deserialize = "kebab-case"))]
   pub struct ArtistSearchResponse {
       #[serde(rename(deserialize = "count"))]
       pub artist_count: usize,
       pub artists: Vec<ArtistSearchResult>,
   }
   ```
   (`count` is the top-level field MusicBrainz returns; the existing private
   `ArtistSearchResponse` struct is promoted and extended.)

### Backend — `routes/mb.rs`

4. `search_artist` handler gains
   `Query(Pagination { page }): Query<Pagination>` (the `Pagination` struct at
   L54-57 is reused as-is).
5. It calls `musicbrainz.search_artist(&query, page)` and returns
   `Json<ArtistSearchResponse>` (was `Json<Vec<ArtistSearchResult>>`).
6. The import on L13 swaps `ArtistSearchResult` → `ArtistSearchResponse`.

`main.rs` is **not touched** — the existing `/mb/search_artist/{query}` route
already accepts `?page=N` because axum pulls `Query` from the URL string.

### Frontend — `artist/add/+page.ts` (NEW)

7. `PageLoad` reads `q = url.searchParams.get("q") ?? ""` and
   `page = Number(url.searchParams.get("page") ?? "0")`.
8. If `q === ""`, returns `{ query: "", page: 0, pages: 0, artists: null }` — no
   fetch; the form-only state.
9. Otherwise fetches
   `${API_URL}/mb/search_artist/${encodeURIComponent(q)}?page=${page}`, parses as
   `ArtistSearchResp`, returns
   `{ query: q, page, pages: Math.ceil(artist_count / 100), artists }`.
10. Inline types `ArtistSearchResp` and `Artist` (matching the existing
    `+page.ts` convention of defining types inline rather than importing `ts_rs`
    bindings).

### Frontend — `artist/add/+page.svelte` (REWRITE)

11. `let { data }: PageProps = $props()` replaces `let query = $state("")` /
    `let results = $state(null)`.
12. Local `query = $state(data.query)` so the input doesn't clear when paginating.
13. `submit()` → `goto(\`/artist/add?q=${encodeURIComponent(query)}&page=0\`)`.
    Navigating to the same route with new search params re-fires `load`.
14. Form is always rendered; the results table renders only when
    `data.artists !== null`; an empty array renders the existing "No results"
    branch.
15. Per-row `addArtist(id)` is unchanged: `POST ${API_URL}/artist/{id}`, then
    `goto(\`/artist/${id}\`)` on success.
16. Pagination controls below the table: Prev (`goto` to `page - 1`), a
    "Page {page + 1} of {pages}" label, Next (`goto` to `page + 1`). Prev
    disabled when `page <= 0`; Next disabled when `page >= pages - 1`.

### Frontend — `add-release-group/[page]/+page.ts` (one-line fix)

17. `pages: Math.floor(release_group_count / 100)` →
    `pages: Math.ceil(release_group_count / 100)`. Adding pagination *controls*
    to that page's `+page.svelte` is out of scope; only the math is corrected.

**Acceptance**
- `cargo build` passes in `platen-backend`.
- `npm run check` passes in `platen-frontend` (or `npm run build` if that's the
  repo's actual gate — confirm before picking up).
- `GET /mb/search_artist/{query}` (no `?page=`) returns the first 100 results
  wrapped in `{ artist_count, artists }`; `?page=1` returns the next 100;
  `artist_count` equals MusicBrainz's reported `count` for that query.
- Visiting `/artist/add` shows only the form; no network request fires.
- Submitting "radiohead" navigates to `/artist/add?q=radiohead&page=0` and
  renders up to 100 results.
- Prev is disabled on page 0; Next is disabled on the last page.
- Clicking a result POSTs and redirects to `/artist/{id}` as before.
- The URL is shareable: pasting `/artist/add?q=radiohead&page=1` loads page 1
  directly.
- For a release-group count that is an exact multiple of 100, the
  `add-release-group` page's `pages` no longer leaves an empty trailing page.

**Dependencies**: none.
