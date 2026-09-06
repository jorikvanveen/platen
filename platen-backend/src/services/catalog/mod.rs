mod matching;
mod reconciliation;
mod scan;
mod utils;

pub(crate) use scan::{ScanCoordinator, ScanPhase, ScanSnapshot, ScanSummary};
pub(crate) use utils::{PrepareAlbumError, persist_album, prepare_album};

#[cfg(test)]
pub(crate) use scan::EmptyTidalCatalog;
