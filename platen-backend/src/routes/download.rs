use crate::{AppState, entity::album, services::download_queue::JobStatus};
use axum::{Json, extract::State};
use reqwest::StatusCode;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

pub mod dto {
    use serde::Serialize;
    use ts_rs::TS;

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    #[serde(rename_all = "lowercase")]
    pub enum DownloadJobStatus {
        Queued,
        Running,
        Succeeded,
        Failed,
        Cancelled,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct DownloadJob {
        pub id: String,
        pub album_id: String,
        pub release_name: String,
        pub status: DownloadJobStatus,
    }
}

impl From<JobStatus> for dto::DownloadJobStatus {
    fn from(status: JobStatus) -> Self {
        match status {
            JobStatus::Queued => Self::Queued,
            JobStatus::Running => Self::Running,
            JobStatus::Succeeded => Self::Succeeded,
            JobStatus::Failed => Self::Failed,
            JobStatus::Cancelled => Self::Cancelled,
        }
    }
}

pub async fn list(
    State(AppState { db, queue, .. }): State<AppState>,
) -> Result<Json<Vec<dto::DownloadJob>>, StatusCode> {
    let jobs = queue.active().await;
    if jobs.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let album_ids: Vec<_> = jobs.iter().map(|job| job.album_id.clone()).collect();
    let albums = album::Entity::find()
        .filter(album::Column::Id.is_in(album_ids))
        .all(&db)
        .await
        .map_err(|error| {
            tracing::error!("Could not load albums for downloads: {error:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let titles = albums
        .into_iter()
        .map(|album| (album.id, album.title))
        .collect::<std::collections::HashMap<_, _>>();

    Ok(Json(
        jobs.into_iter()
            .map(|job| dto::DownloadJob {
                id: job.id.to_string(),
                release_name: titles
                    .get(&job.album_id)
                    .cloned()
                    .unwrap_or_else(|| job.album_id.clone()),
                album_id: job.album_id,
                status: job.status.into(),
            })
            .collect(),
    ))
}
