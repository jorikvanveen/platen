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

#[cfg(test)]
mod tests {
    use sea_orm_migration::{
        sea_orm::{ConnectionTrait, Database, DbBackend, Statement},
        MigratorTrait,
    };

    use crate::Migrator;

    #[async_std::test]
    async fn migration_discards_old_downloaded_values_and_enforces_unique_locations() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, Some(4)).await.unwrap();
        db.execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO album (id, title, album_type, release_year, release_month, release_day, downloaded, cover_url) VALUES ('downloaded', 'Downloaded', NULL, 2026, NULL, NULL, TRUE, NULL), ('pending', 'Pending', NULL, 2026, NULL, NULL, FALSE, NULL)".to_owned(),
        ))
        .await
        .unwrap();

        Migrator::up(&db, None).await.unwrap();

        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT relative_path FROM album ORDER BY id".to_owned(),
            ))
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row
            .try_get::<Option<String>>("", "relative_path")
            .unwrap()
            .is_none()));
        assert!(db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT downloaded FROM album LIMIT 1".to_owned(),
            ))
            .await
            .is_err());

        db.execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "UPDATE album SET relative_path = 'Artist/Album' WHERE id = 'downloaded'".to_owned(),
        ))
        .await
        .unwrap();
        assert!(db
            .execute_raw(Statement::from_string(
                DbBackend::Sqlite,
                "UPDATE album SET relative_path = 'Artist/Album' WHERE id = 'pending'".to_owned(),
            ))
            .await
            .is_err());
    }
}
