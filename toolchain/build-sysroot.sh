#!/usr/bin/env bash
# Builds the egui rlib set that the browser compiler links user programs against, and packs it
# into the same RIWB1 bundle format the std sysroot uses.
#
# The rlibs must be built by a stage1 of the riw fork, with the LLVM backend, because rustc
# running inside wasm cannot load proc-macro dylibs. Only user code goes through clif2wasm.
#
# The rlibs must also be compiled against the same std the browser will link against, so
# `STD_SYSROOT` points at the released wasip1 sysroot, not at the stage1's own.
#
# Env:
#   STAGE1_RUSTC  stage1 rustc of the fork  (default ~/dev/rust/build/host/stage1/bin/rustc)
#   STD_SYSROOT   extracted wasip1 std sysroot
#   OUT_DIR       where the staged sysroot and the bundle land
set -euo pipefail

cd "$(dirname "$0")/.."

STAGE1_RUSTC="${STAGE1_RUSTC:-$HOME/dev/rust/build/host/stage1/bin/rustc}"
STD_SYSROOT="${STD_SYSROOT:?set STD_SYSROOT to the extracted wasip1 std sysroot}"
OUT_DIR="${OUT_DIR:-target/sysroot-egui}"
BUILD_DIR="${BUILD_DIR:-target/toolchain}"

test -x "$STAGE1_RUSTC" || { echo "error: no stage1 rustc at $STAGE1_RUSTC" >&2; exit 1; }
test -d "$STD_SYSROOT/lib/rustlib/wasm32-wasip1/lib" || {
    echo "error: $STD_SYSROOT does not look like a wasip1 sysroot" >&2; exit 1; }

version=$("$STAGE1_RUSTC" --version)
echo "==> stage1: $version"

echo "==> building the rlib set"
# Stripping build script binaries would call `rust-objcopy`, which a stage1 build tree does not
# lay out under that name.
RUSTC="$STAGE1_RUSTC" \
CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="--sysroot $(cd "$STD_SYSROOT" && pwd)" \
CARGO_TARGET_DIR="$BUILD_DIR" \
CARGO_PROFILE_RELEASE_STRIP=false \
    cargo build -p playground-toolchain --target wasm32-wasip1 --release

deps="$BUILD_DIR/wasm32-wasip1/release/deps"
lib="$OUT_DIR/lib/rustlib/wasm32-wasip1/lib"
rm -rf "$OUT_DIR"
mkdir -p "$lib"

# Each rlib carries its own metadata, so the `.rmeta` cargo writes next to it would be a second
# copy of the same bytes and roughly half the bundle.
for rlib in "$deps"/*.rlib; do
    cp "$rlib" "$lib/"
done

python3 - "$OUT_DIR" <<'PY'
import json, os, sys

root = sys.argv[1]
files = []
for dirpath, _dirs, names in os.walk(root):
    for name in sorted(names):
        files.append(os.path.relpath(os.path.join(dirpath, name), root))
json.dump({"files": files}, open(f"{root}/manifest.json", "w"))
print(f"  manifest: {len(files)} files")
PY

python3 - "$OUT_DIR" "$OUT_DIR.bundle" <<'PY'
import json, os, struct, sys

root, out = sys.argv[1], sys.argv[2]
manifest = json.load(open(os.path.join(root, "manifest.json")))

files, blobs, offset = [], [], 0
for rel in manifest["files"]:
    data = open(os.path.join(root, rel), "rb").read()
    files.append({"p": rel, "o": offset, "l": len(data)})
    blobs.append(data)
    offset += len(data)

index = json.dumps({"files": files, "total": offset}).encode()
with open(out, "wb") as f:
    f.write(b"RIWB1\n")
    f.write(struct.pack("<I", len(index)))
    f.write(index)
    for blob in blobs:
        f.write(blob)
print(f"  bundle: {len(files)} files, {(10 + len(index) + offset) / 1e6:.1f} MB -> {out}")
PY

echo "==> externs the compile worker passes:"
for name in egui egui_playground pg_abi; do
    path=$(ls "$lib"/lib$name-*.rlib 2>/dev/null | head -1)
    [ -n "$path" ] && echo "    --extern $name=$(basename "$path")"
done
