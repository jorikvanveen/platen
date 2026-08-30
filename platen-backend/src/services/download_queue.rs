use std::{collections::VecDeque, path::PathBuf, sync::Arc};

use chrono::{DateTime, Utc};
use nanoid::nanoid;
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};

use crate::{routes::album as album_route, services::downloaders::Downloader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    #[allow(dead_code)]
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
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("download worker is not running")]
    WorkerStopped,
}

#[derive(Clone)]
pub struct DownloadQueue {
    state: Arc<Mutex<QueueState>>,
    sender: mpsc::UnboundedSender<String>,
}

const HISTORY_LIMIT: usize = 100;

struct QueueState {
    active: Vec<JobRecord>,
    history: VecDeque<JobRecord>,
}

impl QueueState {
    fn finish(&mut self, index: usize, status: JobStatus, failure_reason: Option<String>) {
        let mut job = self.active.remove(index);
        job.status = status;
        job.finished_at = Some(Utc::now());
        job.failure_reason = failure_reason;
        self.history.push_front(job);
        self.history.truncate(HISTORY_LIMIT);
    }
}

struct DownloadWorker {
    queue: DownloadQueue,
    receiver: mpsc::UnboundedReceiver<String>,
    db: sea_orm::DatabaseConnection,
    music_dir: PathBuf,
    downloader: Arc<dyn Downloader>,
}

impl DownloadQueue {
    pub fn start(
        db: sea_orm::DatabaseConnection,
        music_dir: PathBuf,
        downloader: Arc<dyn Downloader>,
    ) -> (Self, tokio::task::JoinHandle<()>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let queue = Self {
            state: Arc::new(Mutex::new(QueueState {
                active: Vec::new(),
                history: VecDeque::new(),
            })),
            sender,
        };
        let worker = DownloadWorker {
            queue: queue.clone(),
            receiver,
            db,
            music_dir,
            downloader,
        };
        (queue, tokio::spawn(worker.run()))
    }

    pub async fn enqueue(&self, album_id: String) -> Result<JobRecord, QueueError> {
        let mut state = self.state.lock().await;
        if let Some(existing) = state.active.iter().find(|job| job.album_id == album_id) {
            return Ok(existing.clone());
        }

        let record = JobRecord {
            id: nanoid!(),
            album_id,
            status: JobStatus::Queued,
            enqueued_at: Utc::now(),
            started_at: None,
            finished_at: None,
            failure_reason: None,
        };
        let id = record.id.clone();
        state.active.push(record.clone());
        if self.sender.send(id).is_err() {
            state.active.retain(|job| job.id != record.id);
            return Err(QueueError::WorkerStopped);
        }
        Ok(record)
    }

    #[cfg(test)]
    pub async fn active(&self) -> Vec<JobRecord> {
        self.state.lock().await.active.clone()
    }

    pub async fn snapshot(&self) -> (Vec<JobRecord>, Vec<JobRecord>) {
        let state = self.state.lock().await;
        (
            state.active.clone(),
            state.history.iter().cloned().collect(),
        )
    }

    #[allow(dead_code)]
    pub async fn cancel(&self, id: &str) -> bool {
        let mut state = self.state.lock().await;
        let Some(index) = state
            .active
            .iter()
            .position(|job| job.id == id && job.status == JobStatus::Queued)
        else {
            return false;
        };
        state.finish(index, JobStatus::Cancelled, None);
        true
    }

    async fn mark_running(&self, id: &str) -> Option<JobRecord> {
        let mut state = self.state.lock().await;
        let job = state
            .active
            .iter_mut()
            .find(|job| job.id == id && job.status == JobStatus::Queued)?;
        job.status = JobStatus::Running;
        job.started_at = Some(Utc::now());
        Some(job.clone())
    }

    async fn finish(&self, id: &str, status: JobStatus, failure_reason: Option<String>) {
        let mut state = self.state.lock().await;
        let Some(index) = state.active.iter().position(|job| job.id == id) else {
            return;
        };
        state.finish(index, status, failure_reason);
    }
}

impl DownloadWorker {
    async fn run(mut self) {
        while let Some(id) = self.receiver.recv().await {
            let Some(job) = self.queue.mark_running(&id).await else {
                continue;
            };

            let music_dir = self.music_dir.to_string_lossy();
            let result = album_route::download_with(
                &self.db,
                &music_dir,
                self.downloader.as_ref(),
                &job.album_id,
            )
            .await;

            match result {
                Ok(()) => self.queue.finish(&id, JobStatus::Succeeded, None).await,
                Err(error) => {
                    tracing::error!(
                        job_id = %job.id,
                        album_id = %job.album_id,
                        "Download job failed: {error}"
                    );
                    self.queue
                        .finish(
                            &id,
                            JobStatus::Failed,
                            Some(error.client_message().to_owned()),
                        )
                        .await;
                }
            }
        }
    }
}
