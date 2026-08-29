# In-memory single-worker download queue

Status: accepted

Album downloads run through one in-process queue and one background worker. The
HTTP handler creates a Download job and returns without waiting for Antra or
file placement. Each job has an opaque Nano ID and moves through `queued`,
`running`, `succeeded`, `failed`, or `cancelled`.

The worker loads the Album and its Primary artist when it starts a job. That
keeps the destination based on current catalog metadata rather than a snapshot
captured when the user clicked Download. The worker handles database, placement,
and downloader failures as job failures, then continues with the next queued
job. Successful placement marks the Album as downloaded.

Queued and running jobs live only in process memory. Process shutdown aborts
the worker and discards all queued and running jobs. Platen does not recover
those jobs after restart.

## Considered options

- Persistent job storage was rejected because Platen is a single-user,
single-process application and does not currently need restart recovery. A
persistent queue would add a second lifecycle to manage for no current user
benefit.
- Multiple concurrent workers were rejected because Antra downloads and Music
directory placement should remain serialized. One worker also makes execution
order visible and predictable.
- Running the download in the request handler was rejected because a request
would remain open for the full external download and placement operation.

## Consequences

- `POST /albums/{album_id}/download` acknowledges work with `202 Accepted`.
- The Downloads page can show queued and running jobs, but completed jobs are
not retained after they leave the active queue.
- A restart loses queued and running work, so persistent recovery must be
revisited if Platen becomes multi-process or downloads become operationally
important across restarts.
- The single worker limits throughput to one Album at a time by design.
