use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite cannot rename a foreign-key constraint in place; recreate the table.
        // 1. Create the new release_group table with the renamed FK.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE "release_group__new" (
                    "musicbrainz_id" text NOT NULL PRIMARY KEY,
                    "title" text NOT NULL,
                    "artist_id" text NOT NULL,
                    "type" text NOT NULL,
                    "downloaded" boolean NOT NULL,
                    CONSTRAINT "artist-release-group-fk" FOREIGN KEY ("artist_id") REFERENCES "artist" ("musicbrainz_id") ON DELETE CASCADE ON UPDATE CASCADE
                );
                "#,
            )
            .await?;

        // 2. Copy data over.
        manager
            .get_connection()
            .execute_unprepared(
                r#"INSERT INTO "release_group__new" ("musicbrainz_id", "title", "artist_id", "type", "downloaded")
                   SELECT "musicbrainz_id", "title", "artist_id", "type", "downloaded" FROM "release_group";"#,
            )
            .await?;

        // 3. Drop the old table and rename the new one into place.
        manager
            .get_connection()
            .execute_unprepared(r#"DROP TABLE "release_group";"#)
            .await?;
        manager
            .get_connection()
            .execute_unprepared(r#"ALTER TABLE "release_group__new" RENAME TO "release_group";"#)
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Reverse: recreate the table with the original FK name "artist-release-fk".
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE "release_group__old" (
                    "musicbrainz_id" text NOT NULL PRIMARY KEY,
                    "title" text NOT NULL,
                    "artist_id" text NOT NULL,
                    "type" text NOT NULL,
                    "downloaded" boolean NOT NULL,
                    CONSTRAINT "artist-release-fk" FOREIGN KEY ("artist_id") REFERENCES "artist" ("musicbrainz_id") ON DELETE CASCADE ON UPDATE CASCADE
                );
                "#,
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                r#"INSERT INTO "release_group__old" ("musicbrainz_id", "title", "artist_id", "type", "downloaded")
                   SELECT "musicbrainz_id", "title", "artist_id", "type", "downloaded" FROM "release_group";"#,
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(r#"DROP TABLE "release_group";"#)
            .await?;
        manager
            .get_connection()
            .execute_unprepared(r#"ALTER TABLE "release_group__old" RENAME TO "release_group";"#)
            .await?;

        Ok(())
    }
}
