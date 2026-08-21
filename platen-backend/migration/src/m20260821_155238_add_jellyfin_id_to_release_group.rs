use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ReleaseGroup::Table)
                    .add_column(string_null(ReleaseGroup::JellyfinId))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ReleaseGroup::Table)
                    .drop_column(ReleaseGroup::JellyfinId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ReleaseGroup {
    Table,
    JellyfinId,
}
