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
                    .add_column(ColumnDef::new(Album::RelativePath).string().null())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx-album-relative-path")
                    .table(Album::Table)
                    .col(Album::RelativePath)
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .drop_column(Album::Downloaded)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .add_column(
                        ColumnDef::new(Album::Downloaded)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx-album-relative-path")
                    .table(Album::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .drop_column(Album::RelativePath)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Album {
    Table,
    Downloaded,
    RelativePath,
}
