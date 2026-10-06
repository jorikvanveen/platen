# Persist unfinished Download jobs without download checkpoints

Status: accepted and implemented.

Supersedes the memory-only persistence and restart behavior in
[ADR 0006](0006-in-memory-download-queue.md). Its single-worker decision
remains unchanged.

Platen stores unfinished Download jobs in the existing catalog database,
including their identity, queue ordering, retry counters, and retry deadlines.
Actual download execution state remains transient. Recovery restarts the
interrupted download at the front of the queue without incrementing its retry
counter, unless its retry window expired or the Album is already downloaded.
Existing backoff deadlines still apply to jobs already waiting for a retry.

We accept repeated download work rather than saving enough execution state to
resume an Antra job. Only unfinished work persists; the latest 100 terminal jobs
remain process-local history. This reverses ADR 0006's storage decision because
restart recovery is now an explicit requirement.

The database must save admission and cancellation before Platen acknowledges
them. Queue storage failures pause new attempts rather than falling back to
memory, and a failed restore prevents startup. This sacrifices availability
when storage fails to avoid losing work that Platen already accepted.

An Album location already in the catalog fulfills an unfinished Download job.
An existing directory without a recorded location does not prove completion;
Platen leaves it untouched and applies the normal retry policy. The user may
need to run a Music directory scan after an interruption between file placement
and recording the Album location.
