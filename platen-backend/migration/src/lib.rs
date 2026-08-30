pub use sea_orm_migration::prelude::*;

mod m20220101_000001_create_table;
mod m20260829_145526_add_downloaded_to_album;

mod m20260829_184547_add_cover_url_to_album;
mod m20260830_145052_add_profile_image_url_to_artist;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_create_table::Migration),
            Box::new(m20260829_145526_add_downloaded_to_album::Migration),
            Box::new(m20260829_184547_add_cover_url_to_album::Migration),
            Box::new(m20260830_145052_add_profile_image_url_to_artist::Migration),
        ]
    }
}
