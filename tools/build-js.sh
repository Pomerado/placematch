#!/bin/sh
# Build the WebAssembly module and put it beside the JS loader (js/placematch.wasm), where the
# package's Node entry reads it. A release commits that file; main does not track it.
set -eu
cd "$(dirname "$0")/.."
# the module carries source paths in its panic messages: write them without the builder's own
# directories, so every build of one commit is the same bytes
home="${CARGO_HOME:-$HOME/.cargo}"
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$home=/cargo --remap-path-prefix=$(pwd)=/placematch"
export RUSTFLAGS
cargo build --profile wasm --target wasm32-unknown-unknown -p placematch-wasm
cp target/wasm32-unknown-unknown/wasm/placematch_wasm.wasm js/placematch.wasm
echo "js/placematch.wasm: $(wc -c < js/placematch.wasm) bytes"
