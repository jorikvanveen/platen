# Periodic Antra reauthentication

Status: Implemented and verified.

## Agreed behavior

- Keep the initial Antra login during server startup. Failure still prevents
  startup.
- Use a fixed six-hour `const Duration`, with no configuration setting.
- Anchor the schedule to server startup. The first periodic attempt is due at
  T+6h, then T+12h, T+18h, and so on. Login duration, failure, or success does not
  reset the schedule. Restarting the server starts a new schedule.
- When reauthentication is due, wait until the Download queue has no queued or
  running jobs. Succeeded, failed, and cancelled job history does not count.
- The queue may postpone reauthentication indefinitely. Do not interrupt jobs
  or impose a maximum queue-drain wait.
- Keep at most one pending reauthentication. Do not replay ticks that occur
  while waiting for the queue or while an attempt is running.
- If the T+6h attempt waits until T+13h for the queue to empty, perform one login
  then. The next attempt is due at T+18h, not immediately for the missed T+12h.
- Reuse the downloader's existing HTTP client and cookies for login.
- Bound each scheduled login attempt to 30 seconds. This timeout does not apply
  to waiting for the queue to empty and does not change the startup login.
- Log failed or timed-out scheduled attempts and keep the server running.
  Wait for the next scheduled tick rather than retrying sooner.
- Preserve existing download failure and retry behavior. Do not add reactive
  login or special handling of authentication failures.
- Cancel reauthentication on server shutdown, including while waiting for the
  queue or awaiting a login response.
- Accept the rare overlap with a download arriving after the empty check or
  during login. Do not add a pause protocol solely to prevent this race.

## Implementation

- Keep Antra reauthentication in a separate background task. Keep authentication
  out of the generic download worker and downloader trait.
- Use Tokio's `interval_at` with `MissedTickBehavior::Skip`. Continue consuming
  ticks while one reauthentication is pending so an overdue tick cannot trigger
  a second login immediately afterward. Do not implement custom timer arithmetic.
- While reauthentication is due and the queue is busy, check its active-job state
  once per second. Do not poll the queue between scheduled attempts.
- Do not add a queue pause flag or lock the queue across a network request.
  A new job can start after the empty check or during login; this rare overlap
  remains possible.
- Retain the task handle and cancel it alongside the download worker at shutdown.

Polling is the smaller change here because the queue has no idle notification or
pause mechanism. An event-driven waiter would add notifications to queue state
changes. Strict exclusion would require worker coordination or moving login into
the worker. Neither is needed under the accepted tolerance for rare overlap.

## Verification

The scheduler calls the queue and login directly. During review, the generic
callback helper and its eight paused-clock tests were removed at the user's
request to keep the implementation simple. There are no automated scheduler
timing tests in the final implementation.

Five queue-state tests cover empty, queued, running, terminal-history, and
mixed-job states. Existing download and queue tests remain unchanged.

## Verification results

- `cargo test --manifest-path platen-backend/Cargo.toml --workspace --offline --locked`:
  174 tests passed, including five new queue-state tests.
- `cargo check --manifest-path platen-backend/Cargo.toml --workspace --offline --locked`:
  passed.
- `rustfmt --edition 2024 --config skip_children=true --check` on all changed Rust
  files: passed.
- `git diff --check`: passed.

Verification did not contact Antra or use credentials.
