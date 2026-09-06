#!/usr/bin/env sh
sea-orm-cli generate entity --with-serde both -o ./src/entity --database-url "${1:-sqlite://./db.sqlite?mode=rwc}"
