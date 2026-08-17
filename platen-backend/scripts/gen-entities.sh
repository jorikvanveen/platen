#!/usr/bin/env sh
sea-orm-cli generate entity --with-serde both -o ./src/entity --database-url sqlite://./db.sqlite?mode=rwc
