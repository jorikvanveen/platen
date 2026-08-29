use std::{path::PathBuf, sync::Arc};

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

impl JobStatus {
    fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRecord {
    pub id: String,
    pub album_id: String,
    pub status: JobStatus,
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

struct QueueState {
    jobs: Vec<JobRecord>,
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
            state: Arc::new(Mutex::new(QueueState { jobs: Vec::new() })),
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
        if let Some(existing) = state
            .jobs
            .iter()
            .find(|job| job.album_id == album_id && job.status.is_active())
        {
            return Ok(existing.clone());
        }

        let record = JobRecord {
            id: nanoid!(),
            album_id,
            status: JobStatus::Queued,
        };
        let id = record.id.clone();
        state.jobs.push(record.clone());
        if self.sender.send(id).is_err() {
            state.jobs.retain(|job| job.id != record.id);
            return Err(QueueError::WorkerStopped);
        }
        Ok(record)
    }

    pub async fn active(&self) -> Vec<JobRecord> {
        self.state
            .lock()
            .await
            .jobs
            .iter()
            .filter(|job| job.status.is_active())
            .cloned()
            .collect()
    }

    async fn mark_running(&self, id: &str) -> Option<JobRecord> {
        let mut state = self.state.lock().await;
        let job = state
            .jobs
            .iter_mut()
            .find(|job| job.id == id && job.status == JobStatus::Queued)?;
        job.status = JobStatus::Running;
        Some(job.clone())
    }

    async fn set_status(&self, id: &str, status: JobStatus) {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) {
            job.status = status;
        }
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
                Ok(()) => self.queue.set_status(&id, JobStatus::Succeeded).await,
                Err(error) => {
                    tracing::error!(
                        job_id = %job.id,
                        album_id = %job.album_id,
                        "Download job failed: {error}"
                    );
                    self.queue.set_status(&id, JobStatus::Failed).await;
                }
            }
        }
    }
}
