pub use sea_orm_migration::prelude::*;

mod m20220101_000001_create_table;
mod m20260824_180325_rename_musicbrainz_id_columns;
mod m20260825_000001_add_album_release_date;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_create_table::Migration),
            Box::new(m20260824_180325_rename_musicbrainz_id_columns::Migration),
            Box::new(m20260825_000001_add_album_release_date::Migration),
        ]
    }
}
