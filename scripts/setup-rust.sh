#!/usr/bin/env bash
# Use the project toolchain and the component tool's own locked dependencies.
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
rustc --version
rustup component add rustfmt clippy
rustup target add wasm32-unknown-unknown
cargo install cargo-component --version 0.13.2 --locked
cargo component --version
