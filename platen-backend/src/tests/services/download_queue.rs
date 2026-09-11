use super::*;

fn queue_without_worker() -> (DownloadQueue, mpsc::UnboundedReceiver<String>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let queue = DownloadQueue {
        state: Arc::new(Mutex::new(QueueState {
            active: Vec::new(),
            history: VecDeque::new(),
        })),
        sender,
        music_directory: MusicDirectory::new("unused".into()),
    };
    (queue, receiver)
}

#[tokio::test]
async fn is_empty_initially() {
    let (queue, _receiver) = queue_without_worker();

    assert!(queue.is_empty().await);
}

#[tokio::test]
async fn is_not_empty_with_a_queued_job() {
    let (queue, _receiver) = queue_without_worker();
    let job = queue.enqueue("album".into()).await.unwrap();

    assert_eq!(job.status, JobStatus::Queued);
    assert!(!queue.is_empty().await);
}

#[tokio::test]
async fn is_not_empty_with_a_running_job() {
    let (queue, _receiver) = queue_without_worker();
    let job = queue.enqueue("album".into()).await.unwrap();
    let running = queue.mark_running(&job.id).await.unwrap();

    assert_eq!(running.status, JobStatus::Running);
    assert!(!queue.is_empty().await);
}

#[tokio::test]
async fn is_empty_with_terminal_history() {
    let (queue, _receiver) = queue_without_worker();
    for status in [
        JobStatus::Succeeded,
        JobStatus::Failed,
        JobStatus::Cancelled,
    ] {
        let job = queue.enqueue(format!("album-{status:?}")).await.unwrap();
        assert!(!queue.is_empty().await);
        if status == JobStatus::Cancelled {
            queue.cancel(&job.id).await.unwrap();
        } else {
            queue.mark_running(&job.id).await.unwrap();
            queue.finish(&job.id, status, None).await;
        }
        assert!(queue.is_empty().await);
    }

    let (active, history) = queue.snapshot().await;
    assert!(active.is_empty());
    assert_eq!(
        history.iter().map(|job| job.status).collect::<Vec<_>>(),
        vec![
            JobStatus::Cancelled,
            JobStatus::Failed,
            JobStatus::Succeeded
        ]
    );
}

#[tokio::test]
async fn stays_nonempty_until_all_active_jobs_finish() {
    let (queue, _receiver) = queue_without_worker();
    let running = queue.enqueue("running".into()).await.unwrap();
    queue.mark_running(&running.id).await.unwrap();
    let cancelled = queue.enqueue("cancelled".into()).await.unwrap();
    let queued = queue.enqueue("queued".into()).await.unwrap();

    queue.cancel(&cancelled.id).await.unwrap();
    assert!(!queue.is_empty().await);

    queue.finish(&running.id, JobStatus::Succeeded, None).await;
    assert!(!queue.is_empty().await);

    queue.mark_running(&queued.id).await.unwrap();
    assert!(!queue.is_empty().await);

    queue
        .finish(
            &queued.id,
            JobStatus::Failed,
            Some("download failed".into()),
        )
        .await;
    assert!(queue.is_empty().await);
}
