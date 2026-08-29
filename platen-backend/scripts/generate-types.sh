#!/usr/bin/env sh
cd "$(dirname "$0")/.."

# ts-rs only writes files for types that still have exporters, so a retired
# DTO would otherwise survive as stale output. Clear the export directory
# first; everything in it is generated.
rm -rf ../platen-frontend/src/lib/dto

cargo test export_bindings
