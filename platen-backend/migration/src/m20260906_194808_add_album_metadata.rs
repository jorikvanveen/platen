use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .add_column(ColumnDef::new(Album::Explicit).boolean().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .add_column(ColumnDef::new(Album::MediaTags).json().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [Album::MediaTags, Album::Explicit] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Album::Table)
                        .drop_column(column)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Album {
    Table,
    Explicit,
    MediaTags,
}
