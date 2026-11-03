#!/usr/bin/env bash
# Resolve the project directory and build a release component for the pinned target.
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$project_dir"
command -v rustup >/dev/null || { echo 'Install rustup and source ~/.cargo/env first.' >&2; exit 1; }
component_version="$(cargo component --version)"
[[ "$component_version" == *' 0.13.2 '* ]] || { echo 'cargo-component 0.13.2 is required; run bash scripts/setup-rust.sh.' >&2; exit 1; }
rustup target list --installed | grep -Fxq wasm32-unknown-unknown || {
  echo 'The Wasm target is missing; run bash scripts/setup-rust.sh.' >&2; exit 1;
}
cargo component build --locked --release --target wasm32-unknown-unknown
test -s target/wasm32-unknown-unknown/release/iris_fdw.wasm
echo 'Build artifact: target/wasm32-unknown-unknown/release/iris_fdw.wasm'
