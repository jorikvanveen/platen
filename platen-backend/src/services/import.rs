use std::sync::Arc;

use tokio::sync::Mutex;

/// One failed album from an import run.
#[derive(Debug, Clone)]
pub struct Failure {
    pub name: String,
    pub reason: String,
}

/// Counts from a completed import run.
#[derive(Debug, Clone)]
pub struct Summary {
    pub total_scanned: u32,
    pub created: u32,
    pub linked: u32,
    pub skipped: u32,
    pub failed: u32,
    pub failures: Vec<Failure>,
}

/// Snapshot of the import process state, returned by [`ImportTracker::status`]
/// and as the rejection payload of [`ImportTracker::try_begin_import`].
#[derive(Debug, Clone)]
pub struct Status {
    pub running: bool,
    pub last_summary: Option<Summary>,
}

#[derive(Debug, Default)]
struct Inner {
    running: bool,
    last_summary: Option<Summary>,
}

/// Tracks the in-process import run: whether one is running and the last
/// completed run's summary.
///
/// The tracker owns its mutex internally, so callers share it by cloning
/// `AppState` and never see a lock. `running` is true only while a
/// [`RunningGuard`] exists. The guard does not hold the mutex for the import's
/// duration; it locks only briefly to flip `running` on acquire and off on
/// drop/finish. The long Tidal/MusicBrainz awaits run with the lock free, so
/// [`ImportTracker::status`] can observe `running` mid-import.
#[derive(Debug, Default, Clone)]
pub struct ImportTracker {
    state: Arc<Mutex<Inner>>,
}

impl ImportTracker {
    /// Try to start an import. Returns the [`RunningGuard`] when idle, or
    /// `Err(Status)` with `running = true` and the last completed run's summary
    /// when an import is already in flight.
    ///
    /// Uses `lock().await` rather than `try_lock`: a running import only holds
    /// the mutex during its own begin/finish, so a second request waits at most
    /// for that brief window, not for the whole import.
    pub async fn try_begin_import(&self) -> Result<RunningGuard, Status> {
        let mut guard = self.state.lock().await;
        if guard.running {
            return Err(Status {
                running: true,
                last_summary: guard.last_summary.clone(),
            });
        }
        guard.running = true;
        drop(guard);
        Ok(RunningGuard {
            state: Arc::clone(&self.state),
            spent: false,
        })
    }

    /// Current import state. Never blocks for long: an import does not hold the
    /// mutex across its external awaits.
    pub async fn status(&self) -> Status {
        let guard = self.state.lock().await;
        Status {
            running: guard.running,
            last_summary: guard.last_summary.clone(),
        }
    }
}

/// RAII guard that keeps the tracked `running` flag true while alive and clears
/// it on `Drop`. Build it with [`ImportTracker::try_begin_import`].
///
/// The guard owns an `Arc` clone of the shared inner state but does not hold the
/// mutex across the import body. Begin locks only long enough to flip `running`
/// to true; [`RunningGuard::finish`] locks only long enough to write
/// `last_summary` and flip it back. This keeps the mutex free during the
/// minutes-long external awaits so status polls read the state without blocking
/// for the whole import.
#[derive(Debug)]
pub struct RunningGuard {
    state: Arc<Mutex<Inner>>,
    spent: bool,
}

impl RunningGuard {
    /// Record the completed run's summary and clear `running`. Called on the
    /// success path; after this `Drop` is a no-op.
    pub async fn finish(&mut self, summary: Summary) {
        let mut guard = self.state.lock().await;
        guard.last_summary = Some(summary);
        guard.running = false;
        self.spent = true;
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        if self.spent {
            return;
        }
        // Fallback for early returns and panic unwind: clear the flag without
        // holding up the dropping task. `try_lock` is safe here because a
        // running import does not hold the mutex between begin and finish; the
        // only contender is a brief status read, so this virtually always
        // succeeds. If it ever does not, a process restart clears the flag (see
        // ADR 0002).
        if let Ok(mut guard) = self.state.try_lock() {
            guard.running = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The guard does not hold the mutex between `try_begin_import` and `Drop`;
    // it only locks briefly to flip `running` on begin and off on Drop. So
    // reading `running` while the guard is live is a separate `lock().await`,
    // which is safe under the single-threaded `#[tokio::test]` runtime because
    // no lock is held across the await.

    #[tokio::test]
    async fn running_guard_marks_running_while_held() {
        let tracker = ImportTracker::default();
        let _guard = tracker
            .try_begin_import()
            .await
            .expect("begin when idle");
        assert!(
            tracker.status().await.running,
            "running must be true while the guard is held"
        );
    }

    #[tokio::test]
    async fn running_guard_clears_running_on_drop() {
        let tracker = ImportTracker::default();
        {
            let _guard = tracker.try_begin_import().await.expect("begin when idle");
            assert!(
                tracker.status().await.running,
                "running must be true while the guard is held"
            );
        }
        assert!(
            !tracker.status().await.running,
            "running must be false after the guard drops"
        );
    }

    #[tokio::test]
    async fn running_guard_rejects_second_begin_while_held() {
        let tracker = ImportTracker::default();
        let _first = tracker
            .try_begin_import()
            .await
            .expect("first begin when idle");
        let status = tracker
            .try_begin_import()
            .await
            .expect_err("a second begin while running must be rejected");
        assert!(status.running);
        assert!(status.last_summary.is_none());
    }

    #[tokio::test]
    async fn running_guard_allows_reacquire_after_drop() {
        let tracker = ImportTracker::default();
        {
            let _guard = tracker.try_begin_import().await.expect("begin when idle");
        }
        tracker
            .try_begin_import()
            .await
            .expect("a fresh begin must succeed once the guard has dropped");
    }
}
