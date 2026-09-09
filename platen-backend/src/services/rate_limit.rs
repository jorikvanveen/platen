use std::{sync::Arc, time::Duration};

use tokio::{sync::Semaphore, task, time::sleep};

/// Spaces requests across all clones by at least the cooldown.
#[derive(Clone, Debug)]
pub(crate) struct RateLimit {
    semaphore: Arc<Semaphore>,
    cooldown: Duration,
}

impl RateLimit {
    pub(crate) fn new(cooldown: Duration) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(1)),
            cooldown,
        }
    }

    pub(crate) async fn wait(&self) {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .expect("RateLimit owns the semaphore; it cannot be closed while RateLimit is alive");
        let cooldown = self.cooldown;
        task::spawn(async move {
            // Hold the permit through the cooldown, independently of the caller.
            sleep(cooldown).await;
            drop(permit);
        });
    }
}
