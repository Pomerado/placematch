#!/bin/sh
# Build the WebAssembly module and put it beside the JS loader (js/placematch.wasm), where the
# package's Node entry reads it. A release commits that file; main does not track it.
set -eu
cd "$(dirname "$0")/.."
cargo build --profile wasm --target wasm32-unknown-unknown -p placematch-wasm
cp target/wasm32-unknown-unknown/wasm/placematch_wasm.wasm js/placematch.wasm
echo "js/placematch.wasm: $(wc -c < js/placematch.wasm) bytes"
