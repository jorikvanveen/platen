use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Rename the overloaded `musicbrainz_id` columns to names that say which
/// MusicBrainz entity they hold: a release group ID on Album, an artist ID on
/// Artist. See `docs/adr/0001-tidal-as-identity-authority.md`.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Artist::Table)
                    .rename_column(Artist::MusicbrainzId, Artist::MusicbrainzArtistId)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .rename_column(Album::MusicbrainzId, Album::MusicbrainzReleaseGroupId)
                    .to_owned(),
            )
            .await?;

        // Rebuild the per-column indexes to match the new names.
        manager
            .drop_index(Index::drop().name("artist-musicbrainz-id-idx").to_owned())
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("artist-musicbrainz-artist-id-idx")
                    .table(Artist::Table)
                    .col(Artist::MusicbrainzArtistId)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(Index::drop().name("album-musicbrainz-id-idx").to_owned())
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("album-musicbrainz-release-group-id-idx")
                    .table(Album::Table)
                    .col(Album::MusicbrainzReleaseGroupId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("album-musicbrainz-release-group-id-idx")
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

        manager
            .drop_index(
                Index::drop()
                    .name("artist-musicbrainz-artist-id-idx")
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
            .alter_table(
                Table::alter()
                    .table(Album::Table)
                    .rename_column(Album::MusicbrainzReleaseGroupId, Album::MusicbrainzId)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Artist::Table)
                    .rename_column(Artist::MusicbrainzArtistId, Artist::MusicbrainzId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(Iden)]
#[allow(clippy::enum_variant_names)]
enum Artist {
    Table,
    MusicbrainzId,
    MusicbrainzArtistId,
}

#[derive(Iden)]
#[allow(clippy::enum_variant_names)]
enum Album {
    Table,
    MusicbrainzId,
    MusicbrainzReleaseGroupId,
}
