use sea_orm_migration::{prelude::*, schema::*};

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
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Album::Table)
                    .if_not_exists()
                    .col(string(Album::Id).primary_key())
                    .col(string(Album::Title))
                    .col(string_null(Album::AlbumType))
                    .col(integer(Album::ReleaseYear).default(0))
                    .col(integer_null(Album::ReleaseMonth))
                    .col(integer_null(Album::ReleaseDay))
                    .to_owned(),
            )
            .await?;

        // No ON DELETE on either FK: an Artist credited on any Album must not
        // lose its credit rows (or the Album) when the other side goes away.
        manager
            .create_table(
                Table::create()
                    .table(AlbumArtist::Table)
                    .if_not_exists()
                    .col(string(AlbumArtist::AlbumId))
                    .col(string(AlbumArtist::ArtistId))
                    .col(integer(AlbumArtist::Position))
                    .primary_key(
                        Index::create()
                            .name("pk-album-artist")
                            .col(AlbumArtist::AlbumId)
                            .col(AlbumArtist::ArtistId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-album-artist-album-id")
                            .from(AlbumArtist::Table, AlbumArtist::AlbumId)
                            .to(Album::Table, Album::Id),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-album-artist-artist-id")
                            .from(AlbumArtist::Table, AlbumArtist::ArtistId)
                            .to(Artist::Table, Artist::Id),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AlbumArtist::Table).to_owned())
            .await?;
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
}

#[derive(Iden)]
#[allow(clippy::enum_variant_names)]
enum Album {
    Table,
    Id,
    Title,
    AlbumType,
    ReleaseYear,
    ReleaseMonth,
    ReleaseDay,
}

#[derive(Iden)]
enum AlbumArtist {
    Table,
    AlbumId,
    ArtistId,
    Position,
}
