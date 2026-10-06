use super::*;
use futures_util::FutureExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Database, Set};
use std::{
    path::Path,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Semaphore;

async fn database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

async fn queue_without_worker() -> (DownloadQueue, mpsc::UnboundedReceiver<()>) {
    DownloadQueue::restore(database().await, MusicDirectory::new("unused".into()))
        .await
        .unwrap()
}

async fn stored(queue: &DownloadQueue) -> Vec<download_job::Model> {
    download_job::Entity::find()
        .order_by_asc(download_job::Column::Position)
        .all(&queue.db)
        .await
        .unwrap()
}

#[test]
fn backoff_doubles_then_caps_without_overflow() {
    for (counter, seconds) in [
        (1, 30),
        (2, 60),
        (3, 120),
        (4, 240),
        (5, 480),
        (6, 960),
        (7, 1800),
        (148, 1800),
        (u32::MAX, 1800),
    ] {
        assert_eq!(retry_delay(counter), chrono::Duration::seconds(seconds));
    }
}

#[tokio::test]
async fn failures_keep_identity_and_persist_the_counter_and_deadlines() {
    let (queue, _receiver) = queue_without_worker().await;
    let original = queue.enqueue("album".into()).await.unwrap();
    assert_eq!(original.retry_counter, 0);
    assert!(original.first_failed_at.is_none());
    let first_failure = Utc::now();
    let mut failed_at = first_failure;
    for counter in 1..=8 {
        assert_eq!(queue.next_job().await.unwrap().unwrap().id, original.id);
        queue
            .complete_attempt(&original.id, Some("Download failed."), failed_at)
            .await
            .unwrap();
        let job = queue.snapshot().await.0.remove(0);
        assert_eq!(job.id, original.id);
        assert_eq!(job.retry_counter, counter);
        assert_eq!(job.status, JobStatus::Retrying);
        assert_eq!(job.first_failed_at, Some(first_failure));
        assert_eq!(job.retry_expires_at(), Some(first_failure + RETRY_WINDOW));
        assert_eq!(job.next_retry_at, Some(failed_at + retry_delay(counter)));
        assert_eq!(stored(&queue).await[0].retry_counter, counter as i32);
        assert!(!queue.is_empty().await);
        assert!(queue.snapshot().await.1.is_empty());
        let duplicate = queue.enqueue("album".into()).await.unwrap();
        assert_eq!(duplicate, job);
        failed_at = job.next_retry_at.unwrap();
        queue.advance(failed_at).await.unwrap();
    }
}

#[tokio::test]
async fn due_retry_joins_behind_jobs_waiting_during_backoff() {
    let (queue, _receiver) = queue_without_worker().await;
    let retry = queue.enqueue("retry".into()).await.unwrap();
    queue.next_job().await.unwrap();
    let first = queue.enqueue("first".into()).await.unwrap();
    let failed_at = Utc::now();
    queue
        .complete_attempt(&retry.id, Some("failure"), failed_at)
        .await
        .unwrap();
    let second = queue.enqueue("second".into()).await.unwrap();
    queue
        .advance(failed_at + chrono::Duration::seconds(29))
        .await
        .unwrap();
    assert_eq!(queue.snapshot().await.0[0].status, JobStatus::Retrying);
    queue
        .advance(failed_at + chrono::Duration::seconds(30))
        .await
        .unwrap();
    for expected in [first, second, retry] {
        let running = queue.next_job().await.unwrap().unwrap();
        assert_eq!(running.id, expected.id);
        queue
            .complete_attempt(&running.id, None, Utc::now())
            .await
            .unwrap();
    }
    assert!(queue.is_empty().await);
}

#[tokio::test]
async fn cancellation_during_backoff_is_durable_and_a_new_job_starts_at_zero() {
    let (queue, _receiver) = queue_without_worker().await;
    let first = queue.enqueue("album".into()).await.unwrap();
    queue.next_job().await.unwrap();
    assert!(matches!(
        queue.cancel(&first.id).await,
        Err(CancelError::Running)
    ));
    queue
        .complete_attempt(&first.id, Some("failure"), Utc::now())
        .await
        .unwrap();
    let cancelled = queue.cancel(&first.id).await.unwrap();
    assert_eq!(cancelled.status, JobStatus::Cancelled);
    assert_eq!(cancelled.retry_counter, 1);
    assert!(stored(&queue).await.is_empty());
    assert!(queue.is_empty().await);
    let second = queue.enqueue("album".into()).await.unwrap();
    assert_ne!(second.id, first.id);
    assert_eq!(second.retry_counter, 0);
    assert!(second.retry_expires_at().is_none());
}

#[tokio::test]
async fn retry_window_starts_at_first_failure_and_expires_at_exactly_seven_days() {
    let (queue, _receiver) = queue_without_worker().await;
    let job = queue.enqueue("album".into()).await.unwrap();
    queue
        .advance(job.enqueued_at + chrono::Duration::days(10))
        .await
        .unwrap();
    assert!(queue.snapshot().await.1.is_empty());
    queue.next_job().await.unwrap();
    let first_failure = job.enqueued_at + chrono::Duration::days(10);
    queue
        .complete_attempt(&job.id, Some("failure"), first_failure)
        .await
        .unwrap();
    let expires_at = first_failure + chrono::Duration::days(7);
    assert_eq!(queue.snapshot().await.0[0].retry_expires_at(), Some(expires_at));
    queue
        .advance(first_failure + chrono::Duration::days(3))
        .await
        .unwrap();
    assert_eq!(queue.snapshot().await.0.len(), 1);
    queue
        .advance(expires_at - chrono::Duration::seconds(1))
        .await
        .unwrap();
    assert_eq!(queue.snapshot().await.0.len(), 1);
    queue.advance(expires_at).await.unwrap();
    assert!(queue.is_empty().await);
    assert!(stored(&queue).await.is_empty());
    let failed = queue.snapshot().await.1.remove(0);
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(failed.retry_counter, 1);
    assert!(
        failed
            .failure_reason
            .unwrap()
            .contains("Retry window expired")
    );
}

#[tokio::test]
async fn running_attempt_can_finish_after_expiry_but_failure_does_not_retry() {
    for failure in [None, Some("failure")] {
        let (queue, _receiver) = queue_without_worker().await;
        let job = queue.enqueue("album".into()).await.unwrap();
        queue.next_job().await.unwrap();
        let first_failure = Utc::now();
        queue
            .complete_attempt(&job.id, Some("failure"), first_failure)
            .await
            .unwrap();
        queue
            .advance(first_failure + chrono::Duration::seconds(30))
            .await
            .unwrap();
        queue.next_job().await.unwrap();
        let after_expiry = first_failure + RETRY_WINDOW + chrono::Duration::seconds(1);
        queue.advance(after_expiry).await.unwrap();
        assert_eq!(queue.snapshot().await.0[0].status, JobStatus::Running);
        queue
            .complete_attempt(&job.id, failure, after_expiry)
            .await
            .unwrap();
        assert!(queue.is_empty().await);
        let finished = queue.snapshot().await.1.remove(0);
        assert_eq!(
            finished.status,
            if failure.is_some() {
                JobStatus::Failed
            } else {
                JobStatus::Succeeded
            }
        );
        assert_eq!(
            finished.retry_counter,
            if failure.is_some() { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn reopen_recovers_unfinished_jobs_without_execution_state_or_history() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("queue.sqlite").display()
    );
    let db = Database::connect(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let (queue, receiver) =
        DownloadQueue::restore(db.clone(), MusicDirectory::new("unused".into()))
            .await
            .unwrap();
    let delayed = queue.enqueue("delayed".into()).await.unwrap();
    queue.next_job().await.unwrap();
    let failed_at = Utc::now();
    queue
        .complete_attempt(&delayed.id, Some("last error"), failed_at)
        .await
        .unwrap();
    let running = queue.enqueue("running".into()).await.unwrap();
    queue.next_job().await.unwrap();
    let waiting = queue.enqueue("waiting".into()).await.unwrap();
    let cancelled = queue.enqueue("cancelled".into()).await.unwrap();
    queue.cancel(&cancelled.id).await.unwrap();
    let data = serde_json::to_value(stored(&queue).await).unwrap();
    for record in data.as_array().unwrap() {
        for field in [
            "status",
            "started_at",
            "finished_at",
            "antra_job_id",
            "downloaded_bytes",
        ] {
            assert!(record.get(field).is_none());
        }
    }
    drop(receiver);
    drop(queue);
    db.close().await.unwrap();
    let db = Database::connect(&url).await.unwrap();
    let (restored, _receiver) = DownloadQueue::restore(db, MusicDirectory::new("unused".into()))
        .await
        .unwrap();
    let (active, history) = restored.snapshot().await;
    assert!(history.is_empty());
    assert_eq!(active.len(), 3);
    assert!(
        active
            .iter()
            .all(|job| job.started_at.is_none() && job.finished_at.is_none())
    );
    let delayed = active.iter().find(|job| job.id == delayed.id).unwrap();
    assert_eq!(delayed.retry_counter, 1);
    assert_eq!(
        delayed.next_retry_at,
        Some(failed_at + chrono::Duration::seconds(30))
    );
    assert_eq!(delayed.failure_reason.as_deref(), Some("last error"));
    assert_eq!(restored.next_job().await.unwrap().unwrap().id, running.id);
    restored
        .complete_attempt(&running.id, None, Utc::now())
        .await
        .unwrap();
    assert_eq!(restored.next_job().await.unwrap().unwrap().id, waiting.id);
}

#[tokio::test]
async fn recovery_expires_interrupted_retries_without_incrementing_the_counter() {
    let (queue, _receiver) = queue_without_worker().await;
    let job = queue.enqueue("album".into()).await.unwrap();
    queue.next_job().await.unwrap();
    let failure_time = Utc::now();
    queue
        .complete_attempt(&job.id, Some("failure"), failure_time)
        .await
        .unwrap();
    queue
        .advance(failure_time + chrono::Duration::seconds(30))
        .await
        .unwrap();
    queue.next_job().await.unwrap();
    let (restored, _receiver) =
        DownloadQueue::restore(queue.db.clone(), queue.music_directory.clone())
            .await
            .unwrap();
    restored.advance(failure_time + RETRY_WINDOW).await.unwrap();
    assert!(restored.is_empty().await);
    assert_eq!(restored.snapshot().await.1[0].retry_counter, 1);
    assert_eq!(restored.snapshot().await.1[0].status, JobStatus::Failed);
}

#[tokio::test]
async fn terminal_history_is_bounded_and_does_not_persist() {
    let (queue, _receiver) = queue_without_worker().await;
    for index in 0..=HISTORY_LIMIT {
        let job = queue.enqueue(format!("album-{index}")).await.unwrap();
        queue.cancel(&job.id).await.unwrap();
    }
    assert!(queue.is_empty().await);
    assert_eq!(queue.snapshot().await.1.len(), HISTORY_LIMIT);
    assert_eq!(queue.snapshot().await.1[0].album_id, "album-100");
    assert!(stored(&queue).await.is_empty());
    let (restored, _receiver) =
        DownloadQueue::restore(queue.db.clone(), queue.music_directory.clone())
            .await
            .unwrap();
    assert!(restored.snapshot().await.1.is_empty());
}

#[tokio::test]
async fn failed_admission_or_cancellation_does_not_mutate_memory_or_storage() {
    let (queue, _receiver) = queue_without_worker().await;
    queue.db.execute_unprepared("CREATE TRIGGER reject_insert BEFORE INSERT ON download_job BEGIN SELECT RAISE(ABORT, 'test insertion failure'); END").await.unwrap();
    assert!(matches!(
        queue.enqueue("album".into()).await,
        Err(QueueError::Storage(_))
    ));
    assert!(queue.is_empty().await);
    assert!(stored(&queue).await.is_empty());
    queue
        .db
        .execute_unprepared("DROP TRIGGER reject_insert")
        .await
        .unwrap();
    let first = queue.enqueue("first".into()).await.unwrap();
    queue.enqueue("second".into()).await.unwrap();
    let before = stored(&queue).await;
    queue.db.execute_unprepared("CREATE TRIGGER reject_delete BEFORE DELETE ON download_job BEGIN SELECT RAISE(ABORT, 'test deletion failure'); END").await.unwrap();
    assert!(matches!(
        queue.cancel(&first.id).await,
        Err(CancelError::Storage(_))
    ));
    assert_eq!(queue.snapshot().await.0.len(), 2);
    assert!(queue.snapshot().await.1.is_empty());
    assert_eq!(stored(&queue).await, before);
    queue
        .db
        .execute_unprepared("DROP TRIGGER reject_delete")
        .await
        .unwrap();
    queue.cancel(&first.id).await.unwrap();
    assert_eq!(queue.snapshot().await.0.len(), 1);
}

#[tokio::test]
async fn retrying_a_failed_outcome_write_counts_the_attempt_only_once() {
    let (queue, _receiver) = queue_without_worker().await;
    let job = queue.enqueue("album".into()).await.unwrap();
    queue.next_job().await.unwrap();
    let failed_at = Utc::now();
    queue.db.execute_unprepared("CREATE TRIGGER reject_update BEFORE UPDATE ON download_job BEGIN SELECT RAISE(ABORT, 'test update failure'); END").await.unwrap();
    for _ in 0..3 {
        assert!(
            queue
                .complete_attempt(&job.id, Some("failure"), failed_at)
                .await
                .is_err()
        );
        assert_eq!(queue.snapshot().await.0[0].status, JobStatus::Running);
        assert_eq!(queue.snapshot().await.0[0].retry_counter, 0);
        assert_eq!(stored(&queue).await[0].retry_counter, 0);
    }
    queue
        .db
        .execute_unprepared("DROP TRIGGER reject_update")
        .await
        .unwrap();
    queue
        .complete_attempt(&job.id, Some("failure"), failed_at)
        .await
        .unwrap();
    let job = queue.snapshot().await.0.remove(0);
    assert_eq!(job.retry_counter, 1);
    assert_eq!(job.first_failed_at, Some(failed_at));
}

#[tokio::test]
async fn restore_rejects_unreadable_or_invalid_queue_storage() {
    let empty = Database::connect("sqlite::memory:").await.unwrap();
    assert!(
        DownloadQueue::restore(empty, MusicDirectory::new("unused".into()))
            .await
            .is_err()
    );
    let (queue, _receiver) = queue_without_worker().await;
    queue.enqueue("album".into()).await.unwrap();
    queue
        .db
        .execute_unprepared("UPDATE download_job SET retry_counter = -1")
        .await
        .unwrap();
    assert!(
        DownloadQueue::restore(queue.db.clone(), queue.music_directory.clone())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn album_deletion_cancels_delayed_jobs_only_when_the_transaction_commits() {
    let (queue, _receiver) = queue_without_worker().await;
    let job = queue.enqueue("album".into()).await.unwrap();
    queue.next_job().await.unwrap();
    queue
        .complete_attempt(&job.id, Some("failure"), Utc::now())
        .await
        .unwrap();
    let deletion = queue.prepare_album_deletion("album").await.unwrap();
    let transaction = queue.db.begin().await.unwrap();
    deletion.persist(&transaction).await.unwrap();
    transaction.rollback().await.unwrap();
    drop(deletion);
    assert_eq!(queue.snapshot().await.0[0].status, JobStatus::Retrying);
    assert_eq!(stored(&queue).await.len(), 1);
    let deletion = queue.prepare_album_deletion("album").await.unwrap();
    let transaction = queue.db.begin().await.unwrap();
    deletion.persist(&transaction).await.unwrap();
    transaction.commit().await.unwrap();
    deletion.commit();
    assert!(queue.is_empty().await);
    assert!(stored(&queue).await.is_empty());
    assert_eq!(queue.snapshot().await.1[0].status, JobStatus::Cancelled);
}

struct RecordingDownloader {
    calls: AtomicUsize,
    starts: StdMutex<Vec<String>>,
    failures: usize,
    permits: Semaphore,
}

impl RecordingDownloader {
    fn new(failures: usize, permits: usize) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            starts: StdMutex::new(Vec::new()),
            failures,
            permits: Semaphore::new(permits),
        })
    }
}

#[async_trait::async_trait]
impl Downloader for RecordingDownloader {
    async fn download_album(
        &self,
        album: &album::Model,
        destination: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.starts.lock().unwrap().push(album.id.clone());
        self.permits.acquire().await.unwrap().forget();
        if call < self.failures {
            return Err(std::io::Error::other("test failure").into());
        }
        tokio::fs::write(destination.join("track.flac"), b"audio").await?;
        Ok(())
    }
}

struct Task(tokio::task::JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn worker_fixture() -> (
    DownloadQueue,
    mpsc::UnboundedReceiver<()>,
    tempfile::TempDir,
) {
    let db = database().await;
    for id in ["a", "b"] {
        album::ActiveModel {
            id: Set(id.into()),
            title: Set(id.into()),
            release_year: Set(2026),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
        crate::entity::artist::ActiveModel {
            id: Set(id.into()),
            name: Set(id.into()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
        crate::entity::album_artist::ActiveModel {
            album_id: Set(id.into()),
            artist_id: Set(id.into()),
            position: Set(0),
        }
        .insert(&db)
        .await
        .unwrap();
    }
    let music = tempfile::tempdir().unwrap();
    let (mut queue, receiver) =
        DownloadQueue::restore(db, MusicDirectory::new(music.path().to_owned()))
            .await
            .unwrap();
    let wall_time = Utc::now();
    let instant = tokio::time::Instant::now();
    queue.clock =
        Arc::new(move || wall_time + chrono::Duration::from_std(instant.elapsed()).unwrap());
    (queue, receiver, music)
}

async fn wait_for(
    queue: &DownloadQueue,
    predicate: impl Fn(&(Vec<JobRecord>, Vec<JobRecord>)) -> bool,
) {
    let limit = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = queue.snapshot().await;
        if predicate(&snapshot) {
            return;
        }
        assert!(
            std::time::Instant::now() < limit,
            "queue did not reach expected state: {snapshot:?}"
        );
        tokio::task::yield_now().await;
    }
}

fn prevent_automatic_time_advance() -> Task {
    // SQLite uses real threads; their writes must finish before virtual time advances.
    Task(tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    }))
}

#[tokio::test(start_paused = true)]
async fn timer_retries_without_blocking_other_downloads() {
    let _time = prevent_automatic_time_advance();
    let (queue, receiver, _music) = worker_fixture().await;
    let downloader = RecordingDownloader::new(1, 10);
    let first = queue.enqueue("a".into()).await.unwrap();
    queue.enqueue("b".into()).await.unwrap();
    let _worker = Task(queue.spawn(receiver, downloader.clone()));
    wait_for(&queue, |(_, history)| history.len() == 1).await;
    assert_eq!(downloader.calls.load(Ordering::SeqCst), 2);
    assert_eq!(queue.snapshot().await.0[0].id, first.id);
    tokio::time::advance(Duration::from_secs(29)).await;
    assert_eq!(downloader.calls.load(Ordering::SeqCst), 2);
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for(&queue, |(active, history)| {
        active.is_empty() && history.len() == 2
    })
    .await;
    assert_eq!(*downloader.starts.lock().unwrap(), ["a", "b", "a"]);
    assert_eq!(queue.snapshot().await.1[0].retry_counter, 1);
}

#[tokio::test(start_paused = true)]
async fn failed_completion_write_pauses_downloads_without_repeating_the_transfer() {
    let _time = prevent_automatic_time_advance();
    let (queue, receiver, _music) = worker_fixture().await;
    queue.db.execute_unprepared("CREATE TRIGGER reject_delete BEFORE DELETE ON download_job BEGIN SELECT RAISE(ABORT, 'test completion failure'); END").await.unwrap();
    let downloader = RecordingDownloader::new(0, 10);
    queue.enqueue("a".into()).await.unwrap();
    queue.enqueue("b".into()).await.unwrap();
    let _worker = Task(queue.spawn(receiver, downloader.clone()));
    let limit = std::time::Instant::now() + Duration::from_secs(5);
    while album::Entity::find_by_id("a")
        .one(&queue.db)
        .await
        .unwrap()
        .unwrap()
        .relative_path
        .is_none()
    {
        assert!(std::time::Instant::now() < limit);
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(5)).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(downloader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(queue.snapshot().await.0.len(), 2);
    assert!(queue.snapshot().await.1.is_empty());
    queue
        .db
        .execute_unprepared("DROP TRIGGER reject_delete")
        .await
        .unwrap();
    tokio::time::advance(STORAGE_RETRY_DELAY).await;
    wait_for(&queue, |(active, history)| {
        active.is_empty() && history.len() == 2
    })
    .await;
    assert_eq!(downloader.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn login_guard_excludes_running_attempts_but_not_delayed_work() {
    let (queue, receiver, _music) = worker_fixture().await;
    let downloader = RecordingDownloader::new(1, 0);
    queue.enqueue("a".into()).await.unwrap();
    let _worker = Task(queue.spawn(receiver, downloader.clone()));
    wait_for(&queue, |(active, _)| active[0].status == JobStatus::Running).await;
    assert!(queue.pause_downloads().now_or_never().is_none());
    downloader.permits.add_permits(1);
    wait_for(&queue, |(active, _)| {
        active[0].status == JobStatus::Retrying
    })
    .await;
    let login = queue.pause_downloads().await;
    queue.enqueue("b".into()).await.unwrap();
    assert_eq!(downloader.calls.load(Ordering::SeqCst), 1);
    assert!(
        queue
            .snapshot()
            .await
            .0
            .iter()
            .any(|job| job.album_id == "b" && job.status == JobStatus::Queued)
    );
    drop(login);
    wait_for(&queue, |(active, _)| {
        active
            .iter()
            .any(|job| job.album_id == "b" && job.status == JobStatus::Running)
    })
    .await;
}

#[tokio::test]
async fn recovery_recognizes_downloaded_albums_without_touching_files() {
    let (queue, receiver, music) = worker_fixture().await;
    let downloader = RecordingDownloader::new(0, 10);
    let job = queue.enqueue("a".into()).await.unwrap();
    album::ActiveModel {
        id: Set("a".into()),
        relative_path: Set(Some("existing/audio".into())),
        ..Default::default()
    }
    .update(&queue.db)
    .await
    .unwrap();
    let _worker = Task(queue.spawn(receiver, downloader.clone()));
    wait_for(&queue, |(active, history)| {
        active.is_empty() && history.len() == 1
    })
    .await;
    assert_eq!(queue.snapshot().await.1[0].id, job.id);
    assert_eq!(queue.snapshot().await.1[0].status, JobStatus::Succeeded);
    assert_eq!(downloader.calls.load(Ordering::SeqCst), 0);
    assert!(!music.path().join("a").exists());
}

#[tokio::test]
async fn a_full_queue_reserves_capacity_for_the_running_jobs_retry() {
    let (queue, _receiver) = queue_without_worker().await;
    let original = queue.enqueue("running".into()).await.unwrap();
    queue.next_job().await.unwrap();
    let mut state = queue.state.lock().await;
    let mut full = state.clone();
    for index in 1..UNFINISHED_JOB_LIMIT {
        let mut waiting = original.clone();
        waiting.id = format!("job-{index}");
        waiting.album_id = format!("album-{index}");
        full.active.push(waiting);
    }
    queue.save(&mut state, full).await.unwrap();
    drop(state);
    queue
        .complete_attempt(&original.id, Some("failure"), Utc::now())
        .await
        .unwrap();
    assert_eq!(queue.snapshot().await.0.len(), UNFINISHED_JOB_LIMIT);
    assert_eq!(stored(&queue).await.len(), UNFINISHED_JOB_LIMIT);
    let duplicate = queue.enqueue("running".into()).await.unwrap();
    assert_eq!(duplicate.id, original.id);
    assert_eq!(duplicate.status, JobStatus::Retrying);
    assert_eq!(duplicate.retry_counter, 1);
    assert!(matches!(
        queue.enqueue("overflow".into()).await,
        Err(QueueError::Full)
    ));
}

#[tokio::test(start_paused = true)]
async fn retry_window_expires_while_another_download_is_running() {
    let _time = prevent_automatic_time_advance();
    let (queue, receiver, _music) = worker_fixture().await;
    let retry = queue.enqueue("a".into()).await.unwrap();
    queue.next_job().await.unwrap();
    queue
        .complete_attempt(&retry.id, Some("failure"), queue.now())
        .await
        .unwrap();
    queue.enqueue("b".into()).await.unwrap();
    let downloader = RecordingDownloader::new(0, 0);
    let _worker = Task(queue.spawn(receiver, downloader.clone()));
    wait_for(&queue, |(active, _)| {
        active
            .iter()
            .any(|job| job.album_id == "b" && job.status == JobStatus::Running)
    })
    .await;
    tokio::time::advance(Duration::from_secs(7 * 24 * 60 * 60)).await;
    wait_for(&queue, |(_, history)| history.len() == 1).await;
    assert_eq!(queue.snapshot().await.1[0].id, retry.id);
    assert_eq!(queue.snapshot().await.1[0].status, JobStatus::Failed);
    assert_eq!(queue.snapshot().await.0.len(), 1);
    assert!(downloader.starts.lock().unwrap().iter().all(|id| id == "b"));
}

#[tokio::test]
async fn existing_unrecorded_audio_is_left_untouched_and_retried() {
    let (queue, receiver, music) = worker_fixture().await;
    let destination = music.path().join("a/a (2026)");
    tokio::fs::create_dir_all(&destination).await.unwrap();
    tokio::fs::write(destination.join("track.flac"), b"existing audio")
        .await
        .unwrap();
    let downloader = RecordingDownloader::new(0, 10);
    queue.enqueue("a".into()).await.unwrap();
    let _worker = Task(queue.spawn(receiver, downloader.clone()));
    wait_for(&queue, |(active, _)| {
        active[0].status == JobStatus::Retrying
    })
    .await;
    assert_eq!(
        tokio::fs::read(destination.join("track.flac"))
            .await
            .unwrap(),
        b"existing audio"
    );
    assert_eq!(downloader.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        queue.snapshot().await.0[0].failure_reason.as_deref(),
        Some("Album destination already exists.")
    );
}
