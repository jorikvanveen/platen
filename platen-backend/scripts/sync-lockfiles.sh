#!/usr/bin/env sh
set -eu

cd "$(dirname "$0")/.."
cargo update --offline --package platen-backend

cd ../platen-frontend
npm version "$(node -p 'require("./package.json").version')" \
	--allow-same-version --no-git-tag-version --ignore-scripts
