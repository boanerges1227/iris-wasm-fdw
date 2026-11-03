#!/usr/bin/env bash
# Validate a public checkout without depending on ignored local test files.
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
bash build.sh
rustfmt --check --edition 2021 --config skip_children=true src/lib.rs src/convert.rs
cargo clippy --locked --lib -- -D warnings
