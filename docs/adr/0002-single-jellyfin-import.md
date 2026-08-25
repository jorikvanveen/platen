# Single concurrent Jellyfin import

Platen runs as a single self-hosted binary serving one user against one
Jellyfin server, and Import is a minutes-long operation that hits Tidal and
MusicBrainz for every Jellyfin album. Two concurrent imports would redo each
other's work, double the external API load, and race on the same catalog rows.
We guard the `POST /jellyfin/import` handler with an in-process
`tokio::sync::Mutex<ImportState>` owned by an RAII guard whose lifetime spans
the handler body. The guard locks the mutex only briefly in `acquire` and
`finish` (and best-effort in `Drop`), so the lock stays free across the
minutes-long external awaits and a status poll can read `running` mid-import.
A second request while one is running is rejected with `409 Conflict`. There is
no queue and no cross-process lock.

## Considered Options

- **In-process `Mutex<ImportState>` with immediate rejection.** Cheapest
  possible guard, matches the existing `Arc<Mutex<Tidal>>` precedent in
  `AppState`, and the rejection is cheap to explain to the frontend. The guard
  lives for the handler's duration but does not hold the lock continuously;
  `acquire`, `finish`, and `Drop` lock only for the brief flag flip, so a status
  poll can observe `running` during a minutes-long import. Depends on a single
  process; if platen ever runs replicas, two instances could run imports
  simultaneously.
- **Postgres advisory lock.** Survives multi-instance deploys and process
  restarts without bookkeeping. Costs a round trip per acquire and introduces a
  dependency on the database for something that is fundamentally an
  application-level invariant. Premature for a single-binary personal app.
- **Job queue / background task.** `POST` kicks off a job, returns `202`, and
  the frontend polls a job resource. Reshapes the API contract and the frontend
  for a problem we don't have: the synchronous handler is fine for one user, and
  a guard is a smaller change than a job table.
- **Queue and block the second request.** Ties up a request slot for minutes,
  invites client and proxy timeouts, and the second run would just redo the
  first's work. Worse than rejecting.

## Consequences

- A second import attempt during a running import fails fast with `409` and a
  typed `ImportStatus` body, not a timeout or a hang.
- The guard lives in `AppState`, so it resets on process restart. An import in
  flight when the process dies is lost; the catalog is not left in a "running"
  state because nothing persists the flag.
- If platen grows replicas, this stops being sufficient and we'd revisit with a
  Postgres advisory lock or a job table. The API contract (single import, busy
  rejection) does not change; only the mechanism does.
- A status endpoint (`GET /jellyfin/import/status`) and a `last_summary` field
  on the shared DTO exist so any tab can observe a run it didn't initiate,
  including across a page reload during a minutes-long import.
