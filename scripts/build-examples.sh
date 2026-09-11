#!/usr/bin/env bash
# Builds the guest examples for wasm32-wasip1 and stages them for the host to load.
set -eu

cd "$(dirname "$0")/.."

cargo build -p guest-examples --target wasm32-wasip1 --release

out=crates/host/public/examples
mkdir -p "$out"

for wasm in target/wasm32-wasip1/release/*.wasm; do
    name=$(basename "$wasm" .wasm)
    cp "$wasm" "$out/$name.wasm"
    cp "crates/guest-examples/src/bin/$name.rs" "$out/$name.rs"
done

ls -la "$out"
