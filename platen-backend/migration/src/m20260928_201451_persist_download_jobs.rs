use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(DownloadJob::Table)
                    .col(
                        ColumnDef::new(DownloadJob::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(DownloadJob::AlbumId)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(DownloadJob::Position).integer().not_null())
                    .col(
                        ColumnDef::new(DownloadJob::EnqueuedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DownloadJob::RetryCounter)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(DownloadJob::FirstFailedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(DownloadJob::NextRetryAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(ColumnDef::new(DownloadJob::FailureReason).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(DownloadJob::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum DownloadJob {
    Table,
    Id,
    AlbumId,
    Position,
    EnqueuedAt,
    RetryCounter,
    FirstFailedAt,
    NextRetryAt,
    FailureReason,
}
