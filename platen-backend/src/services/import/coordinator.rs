use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::{Mutex, watch};

use super::{model::ScanSnapshot, workflow::ImportWorkflow};
use crate::services::{music_directory::MusicDirectory, tidal::TidalCatalog};

#[derive(Clone)]
pub(crate) struct ScanCoordinator {
    workflow: ImportWorkflow,
    latest: Arc<Mutex<Option<watch::Receiver<ScanSnapshot>>>>,
}

impl ScanCoordinator {
    pub(crate) fn new(
        music_directory: MusicDirectory,
        db: DatabaseConnection,
        tidal: Arc<dyn TidalCatalog>,
    ) -> Self {
        Self {
            workflow: ImportWorkflow::new(music_directory, db, tidal),
            latest: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) async fn start(&self) -> Result<ScanSnapshot, ScanSnapshot> {
        let initial = ScanSnapshot::default();
        let publisher = {
            let mut latest = self.latest.lock().await;
            if let Some(receiver) = latest.as_ref() {
                let snapshot = receiver.borrow().clone();
                if snapshot.phase.is_active() {
                    return Err(snapshot);
                }
            }
            let (publisher, receiver) = watch::channel(initial.clone());
            *latest = Some(receiver);
            publisher
        };
        let workflow = self.workflow.clone();
        let updates = publisher.clone();
        let worker = tokio::spawn(async move {
            workflow
                .run(|snapshot| {
                    updates.send_replace(snapshot);
                })
                .await
        });
        // A panicked worker must not leave every later start permanently rejected.
        tokio::spawn(async move {
            match worker.await {
                Ok(snapshot) => {
                    publisher.send_replace(snapshot);
                }
                Err(error) => {
                    tracing::error!(%error, "Music directory scan worker stopped unexpectedly");
                    publisher.send_modify(|snapshot| {
                        snapshot.fail("Music directory scan stopped unexpectedly.")
                    });
                }
            }
        });
        Ok(initial)
    }

    pub(crate) async fn snapshot(&self) -> Option<ScanSnapshot> {
        self.latest
            .lock()
            .await
            .as_ref()
            .map(|receiver| receiver.borrow().clone())
    }
}

#[cfg(test)]
#[path = "../../tests/services/import/coordinator.rs"]
mod tests;
