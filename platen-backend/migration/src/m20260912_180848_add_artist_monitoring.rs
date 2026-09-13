use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Artist::Table)
                    .add_column(
                        ColumnDef::new(Artist::Monitored)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;

        for column in [
            Artist::MonitoringBaselineInitializedAt,
            Artist::LastCheckAttemptAt,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Artist::Table)
                        .add_column(ColumnDef::new(column).timestamp_with_time_zone().null())
                        .to_owned(),
                )
                .await?;
        }

        manager
            .create_table(
                Table::create()
                    .table(ArtistKnownAlbum::Table)
                    .col(ColumnDef::new(ArtistKnownAlbum::ArtistId).text().not_null())
                    .col(
                        ColumnDef::new(ArtistKnownAlbum::NormalizedTitle)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ArtistKnownAlbum::ReleaseType)
                            .text()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .name("pk-artist-known-album")
                            .col(ArtistKnownAlbum::ArtistId)
                            .col(ArtistKnownAlbum::NormalizedTitle)
                            .col(ArtistKnownAlbum::ReleaseType),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-artist-known-album-artist-id")
                            .from(ArtistKnownAlbum::Table, ArtistKnownAlbum::ArtistId)
                            .to(Artist::Table, Artist::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ArtistKnownAlbum::Table).to_owned())
            .await?;
        for column in [
            Artist::LastCheckAttemptAt,
            Artist::MonitoringBaselineInitializedAt,
            Artist::Monitored,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Artist::Table)
                        .drop_column(column)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Artist {
    Table,
    Id,
    Monitored,
    MonitoringBaselineInitializedAt,
    LastCheckAttemptAt,
}

#[derive(DeriveIden)]
enum ArtistKnownAlbum {
    Table,
    ArtistId,
    NormalizedTitle,
    ReleaseType,
}
