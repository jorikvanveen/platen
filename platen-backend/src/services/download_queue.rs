use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Utc};
use nanoid::nanoid;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    TransactionTrait, sea_query::OnConflict,
};
use thiserror::Error;
use tokio::sync::{Mutex, MutexGuard, Notify, mpsc};

use crate::{
    entity::{album, download_job},
    routes::album as album_route,
    services::{downloaders::Downloader, music_directory::MusicDirectory},
};

const HISTORY_LIMIT: usize = 100;
const UNFINISHED_JOB_LIMIT: usize = 1_000;
const RETRY_WINDOW: chrono::Duration = chrono::Duration::days(7);
const STORAGE_RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Retrying,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRecord {
    pub id: String,
    pub album_id: String,
    pub status: JobStatus,
    pub enqueued_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
    pub retry_counter: u32,
    pub first_failed_at: Option<DateTime<Utc>>,
    pub next_retry_at: Option<DateTime<Utc>>,
}

impl JobRecord {
    pub fn retry_expires_at(&self) -> Option<DateTime<Utc>> {
        self.first_failed_at
            .and_then(|time| time.checked_add_signed(RETRY_WINDOW))
    }

    fn expired(&self, now: DateTime<Utc>) -> bool {
        self.retry_expires_at()
            .is_some_and(|deadline| deadline <= now)
    }
}

impl TryFrom<download_job::Model> for JobRecord {
    type Error = DbErr;

    fn try_from(stored: download_job::Model) -> Result<Self, Self::Error> {
        let job = Self {
            id: stored.id,
            album_id: stored.album_id,
            status: if stored.next_retry_at.is_some() {
                JobStatus::Retrying
            } else {
                JobStatus::Queued
            },
            enqueued_at: stored.enqueued_at.into(),
            started_at: None,
            finished_at: None,
            failure_reason: stored.failure_reason,
            retry_counter: stored
                .retry_counter
                .try_into()
                .map_err(|_| DbErr::Custom("Invalid persisted retry counter".into()))?,
            first_failed_at: stored.first_failed_at.map(Into::into),
            next_retry_at: stored.next_retry_at.map(Into::into),
        };
        if (job.retry_counter == 0) != job.first_failed_at.is_none()
            || (job.first_failed_at.is_some() && job.retry_expires_at().is_none())
            || (job.next_retry_at.is_some() && job.first_failed_at.is_none())
        {
            return Err(DbErr::Custom("Invalid persisted retry deadlines".into()));
        }
        Ok(job)
    }
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("download queue is full")]
    Full,
    #[error("download worker is not running")]
    WorkerStopped,
    #[error("could not save the download queue: {0}")]
    Storage(#[from] DbErr),
}

#[derive(Debug, Error)]
pub enum CancelError {
    #[error("download job was not found")]
    NotFound,
    #[error("download job is already running")]
    Running,
    #[error("could not save the cancellation: {0}")]
    Storage(#[from] DbErr),
}

#[derive(Clone)]
pub struct DownloadQueue {
    state: Arc<Mutex<QueueState>>,
    coordination: Arc<QueueCoordination>,
    db: DatabaseConnection,
    music_directory: MusicDirectory,
    clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl DownloadQueue {
    pub async fn start(
        db: DatabaseConnection,
        music_directory: MusicDirectory,
        downloader: Arc<dyn Downloader>,
    ) -> Result<(Self, tokio::task::JoinHandle<()>), DbErr> {
        let (queue, receiver) = Self::restore(db, music_directory).await?;
        let handle = queue.spawn(receiver, downloader);
        Ok((queue, handle))
    }

    fn spawn(
        &self,
        receiver: mpsc::UnboundedReceiver<()>,
        downloader: Arc<dyn Downloader>,
    ) -> tokio::task::JoinHandle<()> {
        let worker = DownloadWorker {
            queue: self.clone(),
            receiver,
            downloader,
        };
        let scheduler = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = scheduler.run_scheduler() => {},
                _ = worker.run() => {},
            }
        })
    }

    fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    async fn restore(
        db: DatabaseConnection,
        music_directory: MusicDirectory,
    ) -> Result<(Self, mpsc::UnboundedReceiver<()>), DbErr> {
        let active = download_job::Entity::find()
            .order_by_asc(download_job::Column::Position)
            .all(&db)
            .await?
            .into_iter()
            .map(JobRecord::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        if active.len() > UNFINISHED_JOB_LIMIT {
            return Err(DbErr::Custom(
                "Persisted download queue exceeds its capacity".into(),
            ));
        }
        let (coordination, receiver) = QueueCoordination::new();
        Ok((
            Self {
                state: Arc::new(Mutex::new(QueueState {
                    active,
                    history: VecDeque::new(),
                })),
                coordination: Arc::new(coordination),
                db,
                music_directory,
                clock: Arc::new(Utc::now),
            },
            receiver,
        ))
    }

    pub(crate) fn music_directory(&self) -> &MusicDirectory {
        &self.music_directory
    }

    pub(crate) async fn pause_downloads(&self) -> MutexGuard<'_, ()> {
        self.coordination.pause_downloads().await
    }

    pub async fn enqueue(&self, album_id: String) -> Result<JobRecord, QueueError> {
        let mut state = self.state.lock().await;
        if let Some(existing) = state.active.iter().find(|job| job.album_id == album_id) {
            return Ok(existing.clone());
        }
        if self.coordination.worker_stopped() {
            return Err(QueueError::WorkerStopped);
        }
        let now = self.now();
        let mut next = state.clone();
        next.advance(now, &self.db).await?;
        if next.active.len() >= UNFINISHED_JOB_LIMIT {
            return Err(QueueError::Full);
        }
        let record = JobRecord {
            id: nanoid!(),
            album_id,
            status: JobStatus::Queued,
            enqueued_at: now,
            started_at: None,
            finished_at: None,
            failure_reason: None,
            retry_counter: 0,
            first_failed_at: None,
            next_retry_at: None,
        };
        next.active.push(record.clone());
        self.save(&mut state, next).await?;
        Ok(record)
    }

    #[cfg(test)]
    pub async fn is_empty(&self) -> bool {
        self.state.lock().await.active.is_empty()
    }

    pub async fn snapshot(&self) -> (Vec<JobRecord>, Vec<JobRecord>) {
        let state = self.state.lock().await;
        (
            state.active.clone(),
            state.history.iter().cloned().collect(),
        )
    }

    pub async fn cancel(&self, id: &str) -> Result<JobRecord, CancelError> {
        let mut state = self.state.lock().await;
        let index = state
            .active
            .iter()
            .position(|job| job.id == id)
            .ok_or(CancelError::NotFound)?;
        if state.active[index].status == JobStatus::Running {
            return Err(CancelError::Running);
        }
        let mut next = state.clone();
        let record = next.finish(index, JobStatus::Cancelled, None, self.now());
        self.save(&mut state, next).await?;
        Ok(record)
    }

    pub(crate) async fn prepare_album_deletion(
        &self,
        album_id: &str,
    ) -> Result<AlbumDeletion<'_>, CancelError> {
        let state = self.state.lock().await;
        let mut next = state.clone();
        if let Some(index) = next.active.iter().position(|job| job.album_id == album_id) {
            if next.active[index].status == JobStatus::Running {
                return Err(CancelError::Running);
            }
            next.finish(index, JobStatus::Cancelled, None, self.now());
        }
        Ok(AlbumDeletion {
            queue: self,
            state,
            next,
        })
    }

    async fn save(&self, state: &mut QueueState, next: QueueState) -> Result<(), DbErr> {
        let transaction = self.db.begin().await?;
        persist_changes(&transaction, state, &next).await?;
        transaction.commit().await?;
        *state = next;
        self.coordination.wake();
        Ok(())
    }

    async fn advance(&self, now: DateTime<Utc>) -> Result<(), DbErr> {
        let mut state = self.state.lock().await;
        let mut next = state.clone();
        next.advance(now, &self.db).await?;
        if *state != next {
            self.save(&mut state, next).await?;
        }
        Ok(())
    }

    async fn next_job(&self) -> Result<Option<JobRecord>, DbErr> {
        let mut state = self.state.lock().await;
        let mut next = state.clone();
        let now = self.now();
        next.advance(now, &self.db).await?;
        let job = next
            .active
            .iter_mut()
            .find(|job| job.status == JobStatus::Queued)
            .map(|job| {
                job.status = JobStatus::Running;
                job.started_at = Some(now);
                job.clone()
            });
        if *state != next {
            self.save(&mut state, next).await?;
        }
        Ok(job)
    }

    async fn complete_attempt(
        &self,
        id: &str,
        failure: Option<&str>,
        finished_at: DateTime<Utc>,
    ) -> Result<(), DbErr> {
        let mut state = self.state.lock().await;
        let Some(index) = state.active.iter().position(|job| job.id == id) else {
            return Ok(());
        };
        let mut next = state.clone();
        if let Some(reason) = failure {
            let job = &mut next.active[index];
            job.retry_counter = job
                .retry_counter
                .checked_add(1)
                .ok_or_else(|| DbErr::Custom("Download retry counter overflow".into()))?;
            job.first_failed_at.get_or_insert(finished_at);
            job.failure_reason = Some(reason.to_owned());
            if job.expired(finished_at) {
                let reason = expiry_reason(job);
                next.finish(index, JobStatus::Failed, Some(reason), finished_at);
            } else {
                job.status = JobStatus::Retrying;
                job.next_retry_at = Some(
                    finished_at
                        .checked_add_signed(retry_delay(job.retry_counter))
                        .ok_or_else(|| DbErr::Custom("Download retry deadline overflow".into()))?,
                );
            }
        } else {
            next.finish(index, JobStatus::Succeeded, None, finished_at);
        }
        self.save(&mut state, next).await
    }

    async fn run_scheduler(&self) {
        loop {
            if let Err(error) = self.advance(self.now()).await {
                tracing::error!(%error, "Could not advance download queue; retrying storage operation");
                tokio::time::sleep(STORAGE_RETRY_DELAY).await;
                continue;
            }
            let deadline = self
                .state
                .lock()
                .await
                .active
                .iter()
                .filter(|job| job.status != JobStatus::Running)
                .flat_map(|job| [job.next_retry_at, job.retry_expires_at()])
                .flatten()
                .min();
            self.coordination
                .wait_for_reschedule(deadline, self.now())
                .await;
        }
    }
}

// Hold the queue lock through the catalog transaction so dispatch cannot race deletion.
pub(crate) struct AlbumDeletion<'a> {
    queue: &'a DownloadQueue,
    state: MutexGuard<'a, QueueState>,
    next: QueueState,
}

impl AlbumDeletion<'_> {
    pub(crate) async fn persist(&self, transaction: &impl ConnectionTrait) -> Result<(), DbErr> {
        persist_changes(transaction, &self.state, &self.next).await
    }

    pub(crate) fn commit(mut self) {
        *self.state = self.next;
        self.queue.coordination.wake();
    }
}

#[derive(Clone, PartialEq, Eq)]
struct QueueState {
    active: Vec<JobRecord>,
    history: VecDeque<JobRecord>,
}

impl QueueState {
    fn finish(
        &mut self,
        index: usize,
        status: JobStatus,
        failure_reason: Option<String>,
        now: DateTime<Utc>,
    ) -> JobRecord {
        let mut job = self.active.remove(index);
        job.status = status;
        job.finished_at = Some(now);
        job.next_retry_at = None;
        job.failure_reason = failure_reason;
        self.history.push_front(job.clone());
        self.history.truncate(HISTORY_LIMIT);
        job
    }

    async fn advance(&mut self, now: DateTime<Utc>, db: &DatabaseConnection) -> Result<(), DbErr> {
        let due_ids: Vec<_> = self
            .active
            .iter()
            .filter(|job| {
                job.status != JobStatus::Running
                    && (job.expired(now)
                        || job.next_retry_at.is_some_and(|deadline| deadline <= now))
            })
            .map(|job| job.id.clone())
            .collect();
        for id in due_ids {
            let Some(index) = self.active.iter().position(|job| job.id == id) else {
                continue;
            };
            if self.active[index].expired(now) {
                let downloaded = album::Entity::find_by_id(&self.active[index].album_id)
                    .one(db)
                    .await?
                    .is_some_and(|album| album.relative_path.is_some());
                let reason = (!downloaded).then(|| expiry_reason(&self.active[index]));
                self.finish(
                    index,
                    if downloaded {
                        JobStatus::Succeeded
                    } else {
                        JobStatus::Failed
                    },
                    reason,
                    now,
                );
            } else {
                let mut job = self.active.remove(index);
                job.status = JobStatus::Queued;
                job.next_retry_at = None;
                self.active.push(job);
            }
        }
        Ok(())
    }
}

fn retry_delay(retry_counter: u32) -> chrono::Duration {
    chrono::Duration::seconds((30_i64 << retry_counter.saturating_sub(1).min(6)).min(1_800))
}

fn expiry_reason(job: &JobRecord) -> String {
    format!(
        "Retry window expired. Last failure: {}",
        job.failure_reason.as_deref().unwrap_or("Download failed.")
    )
}


fn persisted_job(job: &JobRecord, position: usize) -> Result<download_job::Model, DbErr> {
    Ok(download_job::Model {
        id: job.id.clone(),
        album_id: job.album_id.clone(),
        position: position
            .try_into()
            .map_err(|_| DbErr::Custom("Download queue position overflow".into()))?,
        enqueued_at: job.enqueued_at.fixed_offset(),
        retry_counter: job
            .retry_counter
            .try_into()
            .map_err(|_| DbErr::Custom("Download retry counter overflow".into()))?,
        first_failed_at: job.first_failed_at.map(|time| time.fixed_offset()),
        next_retry_at: job.next_retry_at.map(|time| time.fixed_offset()),
        failure_reason: job.failure_reason.clone(),
    })
}

async fn persist_changes(
    db: &impl ConnectionTrait,
    previous: &QueueState,
    next: &QueueState,
) -> Result<(), DbErr> {
    let mut previous_by_id = previous
        .active
        .iter()
        .enumerate()
        .map(|(position, job)| {
            persisted_job(job, position).map(|stored| (stored.id.clone(), stored))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;
    for (position, job) in next.active.iter().enumerate() {
        let stored = persisted_job(job, position)?;
        if previous_by_id.remove(&stored.id).as_ref() == Some(&stored) {
            continue;
        }
        let active: download_job::ActiveModel = stored.into();
        download_job::Entity::insert(active)
            .on_conflict(
                OnConflict::column(download_job::Column::Id)
                    .update_columns([
                        download_job::Column::Position,
                        download_job::Column::RetryCounter,
                        download_job::Column::FirstFailedAt,
                        download_job::Column::NextRetryAt,
                        download_job::Column::FailureReason,
                    ])
                    .to_owned(),
            )
            .exec(db)
            .await?;
    }
    if !previous_by_id.is_empty() {
        download_job::Entity::delete_many()
            .filter(download_job::Column::Id.is_in(previous_by_id.into_keys()))
            .exec(db)
            .await?;
    }
    Ok(())
}

struct QueueCoordination {
    worker_wakeup: mpsc::UnboundedSender<()>,
    schedule_changed: Notify,
    attempt_lock: Mutex<()>,
}

impl QueueCoordination {
    fn new() -> (Self, mpsc::UnboundedReceiver<()>) {
        let (worker_wakeup, receiver) = mpsc::unbounded_channel();
        (
            Self {
                worker_wakeup,
                schedule_changed: Notify::new(),
                attempt_lock: Mutex::new(()),
            },
            receiver,
        )
    }

    fn worker_stopped(&self) -> bool {
        self.worker_wakeup.is_closed()
    }

    fn wake(&self) {
        // Both tasks must recheck the queue after a committed change.
        let _ = self.worker_wakeup.send(());
        self.schedule_changed.notify_one();
    }

    async fn pause_downloads(&self) -> MutexGuard<'_, ()> {
        self.attempt_lock.lock().await
    }

    async fn wait_for_reschedule(&self, deadline: Option<DateTime<Utc>>, now: DateTime<Utc>) {
        let deadline_elapsed = async {
            match deadline {
                Some(deadline) => {
                    tokio::time::sleep((deadline - now).to_std().unwrap_or_default()).await
                }
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = self.schedule_changed.notified() => {},
            _ = deadline_elapsed => {},
        }
    }
}

struct DownloadWorker {
    queue: DownloadQueue,
    receiver: mpsc::UnboundedReceiver<()>,
    downloader: Arc<dyn Downloader>,
}

impl DownloadWorker {
    async fn run(mut self) {
        loop {
            let attempt_guard = self.queue.pause_downloads().await;
            let job = match self.queue.next_job().await {
                Ok(Some(job)) => job,
                Ok(None) => {
                    drop(attempt_guard);
                    if self.receiver.recv().await.is_none() {
                        return;
                    }
                    continue;
                }
                Err(error) => {
                    drop(attempt_guard);
                    tracing::error!(%error, "Could not prepare download; retrying storage operation");
                    tokio::time::sleep(STORAGE_RETRY_DELAY).await;
                    continue;
                }
            };
            let result = {
                let _music_dir_guard = self.queue.music_directory.lock().await;
                let music_dir = self.queue.music_directory.path().to_string_lossy();
                album_route::download_with(
                    &self.queue.db,
                    &music_dir,
                    self.downloader.as_ref(),
                    &job.album_id,
                )
                .await
            };
            let finished_at = self.queue.now();
            if let Err(error) = &result {
                tracing::error!(job_id = %job.id, album_id = %job.album_id, "Download attempt failed: {error}");
            }
            let failure = result
                .as_ref()
                .err()
                .map(album_route::DownloadError::client_message);
            // Audio may already be published; a storage retry is not another Download attempt.
            while let Err(error) = self
                .queue
                .complete_attempt(&job.id, failure, finished_at)
                .await
            {
                tracing::error!(job_id = %job.id, %error, "Could not save download outcome; downloads paused");
                tokio::time::sleep(STORAGE_RETRY_DELAY).await;
            }
            drop(attempt_guard);
        }
    }
}

#[cfg(test)]
#[path = "../tests/services/download_queue.rs"]
mod tests;
