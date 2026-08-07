use sea_orm_migration::{prelude::*, schema::*, sea_query::ForeignKeyAction::Cascade};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Replace the sample below with your own migration scripts
        manager
            .create_table(
                Table::create()
                    .table(Artist::Table)
                    .if_not_exists()
                    .col(string(Artist::MusicbrainzId).primary_key())
                    .col(string(Artist::Name))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Release::Table)
                    .if_not_exists()
                    .col(string(Release::MusicbrainzId).primary_key())
                    .col(string(Release::Title))
                    .col(string(Release::ArtistId))
                    .col(boolean(Release::Downloaded))
                    .foreign_key(
                        ForeignKey::create()
                            .name("artist-release-fk")
                            .from(Release::Table, Release::ArtistId)
                            .to(Artist::Table, Artist::MusicbrainzId)
                            .on_delete(Cascade)
                            .on_update(Cascade)
                    )
                    .to_owned()
            ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Replace the sample below with your own migration scripts
        manager
            .drop_table(Table::drop().table(Artist::Table).to_owned())
            .await?;
        
        manager
            .drop_table(Table::drop().table(Release::Table).to_owned())
            .await?;
        
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Artist {
    Table,
    MusicbrainzId,
    Name,
}

#[derive(DeriveIden)]
enum Release {
    Table,
    MusicbrainzId,
    Title,
    ArtistId,
    Downloaded
}
