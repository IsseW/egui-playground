#!/usr/bin/env bash
# Assembles one artifacts Release from the two `wasm-rustc` runs that produced it, then prints the
# lines to paste into `toolchain/artifacts.lock`.
#
# The compiler and the sysroots come from separate runs whenever the sysroot half is re-run with
# `skip_rustc=true`, which leaves the release job skipped.
#
# Usage: MAIN_RUN=<run id> SYSROOT_RUN=<run id> TAG=<tag> scripts/publish-artifacts.sh
set -euo pipefail

REPO="${REPO:-IsseW/wasm-rustc}"
MAIN_RUN="${MAIN_RUN:?set MAIN_RUN to the run id that built rustc.wasm}"
SYSROOT_RUN="${SYSROOT_RUN:?set SYSROOT_RUN to the run id that built the sysroots}"
TAG="${TAG:?set TAG, for example egui-2026-09-11}"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "==> downloading from $REPO"
gh run download "$MAIN_RUN" --repo "$REPO" --dir "$work/main"
gh run download "$SYSROOT_RUN" --repo "$REPO" --dir "$work/sysroot"

mkdir -p "$work/out"
find "$work/main" "$work/sysroot" -name '*.tar.zst' -exec cp {} "$work/out/" \;

cd "$work/out"
for asset in rustc-wasm.tar.zst std-sysroot.tar.zst wasip1-sysroot.tar.zst egui-sysroot.tar.zst; do
    test -f "$asset" || { echo "error: $asset is missing" >&2; exit 1; }
done
shasum -a 256 *.tar.zst > SHA256SUMS

echo "==> assets"
ls -lh *.tar.zst

echo "==> creating release artifacts-$TAG"
gh release create "artifacts-$TAG" \
    --repo "$REPO" \
    --title "artifacts-$TAG" \
    --notes "rustc.wasm and the std, riscv64 and egui sysroots. Paste SHA256SUMS into the playground's artifacts.lock." \
    *.tar.zst SHA256SUMS

echo
echo "==> paste into toolchain/artifacts.lock"
echo "repo=$REPO"
echo "tag=artifacts-$TAG"
for asset in rustc-wasm.tar.zst std-sysroot.tar.zst wasip1-sysroot.tar.zst egui-sysroot.tar.zst; do
    echo "$asset sha256=$(shasum -a 256 "$asset" | cut -d' ' -f1)"
done
