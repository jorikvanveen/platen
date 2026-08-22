use reqwest::StatusCode;
use serde::Deserialize;
use tracing::error;

use crate::services::tidal::TidalError;

#[derive(Debug, Deserialize)]
pub struct Pagination {
    pub page: usize,
}

pub(crate) fn map_tidal_error(e: TidalError) -> StatusCode {
    match e {
        TidalError::UnexpectedResponse => StatusCode::INTERNAL_SERVER_ERROR,
        TidalError::Reqwest(err) => {
            error!("Reqwest: {err:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
        TidalError::AuthenticationFailed(status, body) => {
            error!("Tidal auth: {status}: {body}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
