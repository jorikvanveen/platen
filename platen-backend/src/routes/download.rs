use crate::{
    app::AppState,
    entity::album,
    services::{
        catalog,
        download_queue::{CancelError, JobRecord, JobStatus},
        tidal,
    },
};
use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::StatusCode;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

pub mod dto {
    use chrono::{DateTime, Utc};
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
        pub release_name: Option<String>,
        pub explicit: Option<bool>,
        pub available_quality: Option<String>,
        pub status: DownloadJobStatus,
        pub enqueued_at: DateTime<Utc>,
        pub started_at: Option<DateTime<Utc>>,
        pub finished_at: Option<DateTime<Utc>>,
        pub failure_reason: Option<String>,
    }

    #[derive(Debug, Serialize, TS)]
    #[ts(export)]
    pub struct Downloads {
        pub active: Vec<DownloadJob>,
        pub history: Vec<DownloadJob>,
    }
}

impl dto::DownloadJob {
    pub(crate) fn from_record(job: JobRecord, album: Option<&album::Model>) -> Self {
        let media_tags = album.and_then(catalog::parse_media_tags);
        Self {
            id: job.id,
            album_id: job.album_id,
            release_name: album.map(|album| album.title.clone()),
            explicit: album.and_then(|album| album.explicit),
            available_quality: tidal::available_quality(media_tags.as_deref()).map(str::to_owned),
            status: job.status.into(),
            enqueued_at: job.enqueued_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
            failure_reason: job.failure_reason,
        }
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

pub async fn cancel(
    State(AppState { db, queue, .. }): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<dto::DownloadJob>, StatusCode> {
    let job = queue.cancel(&job_id).await.map_err(|error| match error {
        CancelError::NotFound => StatusCode::NOT_FOUND,
        CancelError::Running => StatusCode::CONFLICT,
    })?;
    let album = match album::Entity::find_by_id(&job.album_id).one(&db).await {
        Ok(album) => album,
        Err(error) => {
            tracing::error!("Could not load album for cancelled download: {error:#?}");
            None
        }
    };

    Ok(Json(dto::DownloadJob::from_record(job, album.as_ref())))
}

pub async fn list(
    State(AppState { db, queue, .. }): State<AppState>,
) -> Result<Json<dto::Downloads>, StatusCode> {
    let (active, history) = queue.snapshot().await;
    let album_ids: Vec<_> = active
        .iter()
        .chain(history.iter())
        .map(|job| job.album_id.clone())
        .collect();
    let albums = album::Entity::find()
        .filter(album::Column::Id.is_in(album_ids))
        .all(&db)
        .await
        .map_err(|error| {
            tracing::error!("Could not load albums for downloads: {error:#?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let albums_by_id = albums
        .into_iter()
        .map(|album| (album.id.clone(), album))
        .collect::<std::collections::HashMap<_, _>>();

    let map_job = |job: JobRecord| {
        let album = albums_by_id.get(&job.album_id);
        dto::DownloadJob::from_record(job, album)
    };

    Ok(Json(dto::Downloads {
        active: active.into_iter().map(&map_job).collect(),
        history: history.into_iter().map(map_job).collect(),
    }))
}
