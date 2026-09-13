# Always-updated Monitoring Baselines

This audit records the user's clarification after the issue #54 implementation:

> All artists must have their baseline initialized and updated regardless of
> monitoring status. The sole purpose of the monitoring status is to download
> when a new album is found, not to pause/resume baseline tracking.

The user also confirmed that "newly released" means "newly observed". It does not
introduce a release-date eligibility rule.

Corrections to GitHub issues #51, #54, and #55 have been published and verified by
reading the bodies back. Titles, states, labels, and assignees were unchanged.
The worker still implements initialization only. Its retry timestamp storage has
been refined as described below. The glossary reflects the clarified domain rule,
and pre-handoff failure policy remains explicitly unresolved.

## Required behavior

- Every stored Catalog Artist receives background Monitoring Baseline
  initialization and subsequent updates, whether monitored or not.
- The first successful discography observation seeds known entries without
  Catalog additions or downloads. A successful empty observation is valid.
- Later successful observations accumulate known title-and-type entries. A
  preference change, disappearance from Tidal, or Territory change never clears
  previously known entries.
- The preference gates automatic Catalog addition and Download queue handoff,
  not discography fetching or remembering what was observed.
- An Album observed while downloads are disabled becomes known without being
  added or queued. Enabling later does not download that already-known entry.
- Eligibility uses the preference when processing a successful observation, not
  when its fetch started. If disabled then, record the observation without
  automatic work. If re-enabled before processing, use the normal enabled rule.
  Revalidate before automatic work; already accepted jobs are unaffected.
- "New" still means first observed, not released after enabling. An Album that
  appeared during a disabled period but was not observed until after enabling
  can qualify; there is no release-date or insertion-time snapshot.
- Failed fetches leave known entries intact and must not be treated as successful
  empty observations.

Keep the existing six-hour periodic cadence, but apply it to all stored Artists,
including overdue startup work. The corrected spec makes preference
changes independent of check timing, removing enable-triggered checks as well as
disable-triggered pauses.

## Audit findings and spec corrections

- The original [#51](https://github.com/jorikvanveen/platen/issues/51), especially
  user stories 20 through 24 and "Artist preference, checking, and queue handoff",
  limited periodic and startup checks to Monitored Artists, stopped checks when
  disabled, and promised catch-up on re-enabling. The correction replaces those
  rules with continuous tracking and download-only opt-in.
- [#54](https://github.com/jorikvanveen/platen/issues/54) correctly requires
  initialization for every Artist, but explicitly defers all periodic discovery
  to #55. Its implementation is an initialization-only foundation, not an
  always-updated Monitoring Baseline. The revised slice makes clear that #55
  must update every Artist, not only Monitored Artists.
- The [current worker](../platen-backend/src/services/monitoring.rs#L29-L48)
  selects only Artists whose initialization-completion timestamp is null. Once
  initialized, an Artist is never fetched by this worker again, regardless of
  its preference.
- The [initialization-only test](../platen-backend/src/tests/monitoring.rs#L300-L315)
  supplies a later discography and advances ten days, then explicitly expects no
  schedule or history updates. Preserve its initialization and preference
  assertions, but replace that initialization-only expectation when implementing
  ongoing updates.
- The original [#55](https://github.com/jorikvanveen/platen/issues/55) repeated
  monitored-only scheduling and disabled-period catch-up and made known history
  depend on successful download handoff. Its revision removes those assumptions
  and marks the replacement pre-handoff failure policy as an open decision.
- [#56](https://github.com/jorikvanveen/platen/issues/56) retains empty Artists
  based on monitoring status. That is another purpose assigned to the preference,
  separate from baseline scheduling. Taking "sole purpose" literally also requires
  a status-independent Artist lifetime rule; this audit does not silently change
  deletion behavior.
- The former glossary definition made baseline growth depend on Downloaded Album
  status or queue acceptance. [The glossary](../CONTEXT.md#L60-L87) now defines
  accumulated observations independently of monitoring preference and download
  outcomes, and defines "Newly observed Album" without release-date gating.

## Request pacing and implementation choices

`RateLimit` spaces Tidal request starts by one second within the running process.
Tidal also retries certain request failures. Neither mechanism schedules a whole
Artist discography attempt or persists its timing across restarts.

The worker records
[`last_check_attempt_at`](../platen-backend/src/services/monitoring.rs#L63-L68)
before fetching and derives the next deadline as that timestamp plus six hours.
This prevents a failed or interrupted attempt from repeating at each one-minute
worker sweep or being immediately retried after restart.

[`monitoring_baseline_initialized_at`](../platen-backend/src/services/monitoring.rs#L76-L98)
remains a separate one-time marker, written atomically with known entries only
after a successful observation. A successful empty observation also initializes
it. The two timestamps represent initialization success and the last attempt,
rather than storing a derived deadline.

The user approved rewriting the existing uncommitted migration with these fields
and will handle conflicts in previously migrated databases. No compatibility
migration or existing-database repair was added. Fresh migration, populated
Catalog preservation, and a down/up round trip were verified once; entities were
regenerated with the project script. The retained
[interrupted-attempt test](../platen-backend/src/tests/monitoring.rs#L321-L407)
reopens file-backed SQLite and verifies retry eligibility at the six-hour boundary.

The user requested a simpler single-worker flow. The worker selects due Artists
once, records each attempt, fetches the discography, and persists the successful
observation. Conditional attempt claiming and special detection of an Artist
deleted and reintroduced during the fetch were removed, together with that
race-specific test. History and initialization writes remain transactional.

The whole-discography timeout belongs to
[Tidal's `get_artist_albums` operation](../platen-backend/src/services/tidal.rs#L303-L358),
not the monitoring worker. It covers authentication, every page, retries, and
Retry-After sleeps under one ten-minute deadline, shared by all callers. A timeout
returns a Tidal error rather than partial Albums; monitoring handles that error
through its ordinary failed-fetch retry path.

The six-hour retry delay for initialization, one-minute worker sweep, and
ten-minute timeout for a whole discography fetch are implementation choices.
#54 requires later retries but does not prescribe these values. The six-hour
initialization retry reuses the later periodic-check cadence; it is not a
requirement imposed by `RateLimit`.

## Unresolved handoff policy

The old specs used unknown entries to retry failed Catalog additions or rejected
queue submissions. They also prohibited monitoring-owned pending-download state.
Once successful observation always makes an entry known, that same history can
no longer double as a record of unfinished handoff.

Two choices remain:

1. Record every observed entry and make automatic handoff a one-time attempt.
   Failed Catalog additions or rejected submissions are not retried by later
   checks. Accepted jobs still belong to the existing Download queue.
2. Record every observed entry and track unfinished handoff separately.
   This preserves pre-handoff retries but changes the explicit no-pending-state
   scope of #51 and #55.

Do not choose between losing pre-handoff retries and introducing separate state
without user agreement. Neither choice may pause or roll back baseline tracking.

## Verification needed for ongoing updates

- Both preference values receive initial, periodic, and overdue startup updates.
- Initialization creates no back-catalog additions or Download jobs.
- A new disabled-period observation becomes known without adding or queueing it,
  and enabling does not turn it into a new discovery.
- An enabled Artist's genuinely new observation can trigger the normal handoff.
- An old release first observed while enabled can qualify, including one that
  appeared while disabled but was not observed until after enabling.
- Remaining disabled through processing a gated fetch suppresses handoff but
  preserves the observation. Re-enabling before processing uses the enabled rule.
- Repeated, temporarily absent, renamed, and same-title/different-type entries
  follow the existing shared identity rule without resetting history.
- Fetch failures preserve history, and retries follow persisted timing.
- Failed handoffs follow the explicitly chosen policy independently of history.
