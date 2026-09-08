mod coordinator;
mod discovery;
mod matching;
mod model;
mod reconciliation;
mod repository;
mod workflow;

pub(crate) use coordinator::ScanCoordinator;
pub(crate) use model::{ScanPhase, ScanSnapshot, ScanSummary};
