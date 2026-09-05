use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::sync::{Mutex, MutexGuard};

#[derive(Clone)]
pub(crate) struct MusicDirectory {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl MusicDirectory {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) async fn lock(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().await
    }
}
