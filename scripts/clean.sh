#!/usr/bin/env bash
# Removes moodbeat build artifacts: Rust/Tauri output (core/target), generated
# Tauri schemas (core/gen/schemas) and the Vite build (dist).
# Pass --all to also remove node_modules.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if command -v cargo >/dev/null 2>&1; then
  echo "Cleaning core/target"
  cargo clean --manifest-path "$ROOT/core/Cargo.toml"
else
  echo "cargo not found, removing core/target directly"
  rm -rf "$ROOT/core/target"
fi

for dir in dist core/gen/schemas; do
  echo "Removing $dir"
  rm -rf "${ROOT:?}/$dir"
done

if [[ "${1:-}" == "--all" ]]; then
  echo "Removing node_modules"
  rm -rf "${ROOT:?}/node_modules"
fi
