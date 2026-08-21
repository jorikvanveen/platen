use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Pagination {
    pub page: usize,
}
