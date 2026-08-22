use sea_orm_migration::{prelude::*, schema::*, sea_query::ForeignKeyAction::Cascade};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Artist::Table)
                    .if_not_exists()
                    .col(string(Artist::Id).primary_key())
                    .col(string(Artist::Name))
                    .col(string_null(Artist::MusicbrainzId))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("artist-musicbrainz-id-idx")
                    .table(Artist::Table)
                    .col(Artist::MusicbrainzId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Album::Table)
                    .if_not_exists()
                    .col(string(Album::Id).primary_key())
                    .col(string(Album::ArtistId))
                    .col(string(Album::Title))
                    .col(string_null(Album::AlbumType))
                    .col(string_null(Album::JellyfinId))
                    .col(string_null(Album::MusicbrainzId))
                    .col(string_null(Album::MatchMethod))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-album-artist-id")
                            .from(Album::Table, Album::ArtistId)
                            .to(Artist::Table, Artist::Id)
                            .on_delete(Cascade)
                            .on_update(Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("album-musicbrainz-id-idx")
                    .table(Album::Table)
                    .col(Album::MusicbrainzId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Album::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Artist::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(Iden)]
enum Artist {
    Table,
    Id,
    Name,
    MusicbrainzId,
}

#[derive(Iden)]
enum Album {
    Table,
    Id,
    ArtistId,
    Title,
    AlbumType,
    JellyfinId,
    MusicbrainzId,
    MatchMethod,
}
