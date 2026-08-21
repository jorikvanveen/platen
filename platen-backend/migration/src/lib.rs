pub use sea_orm_migration::prelude::*;

mod m20220101_000001_create_table;
mod m20260820_170510_rename_release_group_fk;
mod m20260821_155238_add_jellyfin_id_to_release_group;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_create_table::Migration),
            Box::new(m20260820_170510_rename_release_group_fk::Migration),
            Box::new(m20260821_155238_add_jellyfin_id_to_release_group::Migration),
        ]
    }
}
