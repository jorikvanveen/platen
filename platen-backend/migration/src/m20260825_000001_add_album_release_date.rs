use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .add_column(integer(Album::ReleaseYear).default(0))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .add_column(integer_null(Album::ReleaseMonth))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .add_column(integer_null(Album::ReleaseDay))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .drop_column(Album::ReleaseDay)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .drop_column(Album::ReleaseMonth)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .drop_column(Album::ReleaseYear)
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
enum Album {
    Table,
    ReleaseYear,
    ReleaseMonth,
    ReleaseDay,
}
