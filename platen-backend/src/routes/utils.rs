use reqwest::StatusCode;
use tracing::error;

use crate::services::tidal::TidalError;

pub(crate) fn map_tidal_error(e: TidalError) -> StatusCode {
    error!("Tidal error: {e:#?}");
    StatusCode::INTERNAL_SERVER_ERROR
}
