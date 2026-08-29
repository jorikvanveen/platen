#!/usr/bin/env sh
cd "$(dirname "$0")/.."

# Remove stale DTOs because ts-rs only exports types that still have exporters.
rm -rf ../platen-frontend/src/lib/dto

cargo test export_bindings
