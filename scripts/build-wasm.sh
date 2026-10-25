#!/usr/bin/env bash
# Builds the scoring engine to WASM and generates the JS glue into web/src/lib/wasm/ (git-ignored).
set -euo pipefail

cd "$(dirname "$0")/.."

# Must match the exact wasm-bindgen version pinned in crates/scoring-wasm/Cargo.toml.
WANTED="$(sed -n 's/^wasm-bindgen = "=\(.*\)"$/\1/p' crates/scoring-wasm/Cargo.toml)"
if [ -z "$WANTED" ]; then
  echo "error: could not read the pinned wasm-bindgen version from crates/scoring-wasm/Cargo.toml" >&2
  exit 1
fi
INSTALL="cargo install wasm-bindgen-cli --version $WANTED --locked"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "error: wasm-bindgen-cli is not installed. Run: $INSTALL" >&2
  exit 1
fi
HAVE="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$HAVE" != "$WANTED" ]; then
  echo "error: wasm-bindgen-cli $HAVE is installed but $WANTED is required. Run: $INSTALL" >&2
  exit 1
fi

OUT=web/src/lib/wasm
mkdir -p "$OUT"
cargo build -p padel-scoring-wasm --release --target wasm32-unknown-unknown --locked
wasm-bindgen --target web --out-dir "$OUT" target/wasm32-unknown-unknown/release/padel_scoring_wasm.wasm
echo "wasm bindings written to $OUT"
