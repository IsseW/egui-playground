//! `cargo xtask prep` stages the pinned browser compiler artifacts into
//! `crates/host/public/rustc/`.
//!
//! Ported from weblings (MIT), <https://github.com/AngelOnFira/weblings>.
//!
//! Modes:
//!   (default)            stage only what the site loads: the stripped `rustc.wasm`, the
//!                        `sysroot-wasip1.bundle` and `sysroot-egui.bundle` single file
//!                        bundles, and `assets-meta.json`.
//!   --full               also extract the loose sysroot trees and the x86_64 `std-sysroot`.
//!   --public DIR         stage into DIR instead of `crates/host/public`.
//!
//! `RIW_ARTIFACTS_LOCAL=<other checkout's public/>` copies from that directory instead of
//! downloading. `RIW_ARTIFACTS_CACHE` moves the download cache.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use xtask::*;

fn main() {
    if let Err(e) = run() {
        eprintln!("xtask: error: {e:#}");
        std::process::exit(1);
    }
}

fn repo_root() -> PathBuf {
    // `xtask/` lives at the repo root, and `CARGO_MANIFEST_DIR` holds regardless of cwd.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run() -> Result<()> {
    let (flags, pos) = parse_args(std::env::args().skip(1), &["public"]);
    match pos.first().map(String::as_str) {
        Some("prep") => prep(&flags),
        Some(other) => bail!("unknown command `{other}`, expected `prep`"),
        None => bail!("usage: cargo xtask prep [--full] [--public DIR]"),
    }
}

fn prep(flags: &std::collections::HashMap<String, String>) -> Result<()> {
    let root = repo_root();
    let full = flags.contains_key("full");
    let public = flags
        .get("public")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("crates/host/public"));
    let rustc_dir = public.join("rustc");

    // Everything staged already, so `trunk serve` rebuilds stay quick.
    if !full
        && rustc_dir.join("rustc.wasm").exists()
        && rustc_dir.join("sysroot-wasip1.bundle").exists()
        && rustc_dir.join("sysroot-egui.bundle").exists()
        && rustc_dir.join("assets-meta.json").exists()
    {
        return Ok(());
    }

    if let Ok(local) = std::env::var("RIW_ARTIFACTS_LOCAL") {
        return stage_local(Path::new(&local), &public, full);
    }

    let lock = parse_lock(&root.join("toolchain/artifacts.lock"))?;
    eprintln!("xtask: staging artifacts from {}@{}", lock.repo, lock.tag);
    fs::create_dir_all(&rustc_dir)?;

    for (name, sha) in &lock.assets {
        match name.as_str() {
            "rustc-wasm.tar.zst" => {
                let asset = fetch_asset(&lock, name, sha)?;
                let got = extract_tar_zst(&asset, |p| {
                    (p.file_name()?.to_str()? == "rustc.wasm").then(|| rustc_dir.join("rustc.wasm"))
                })?;
                ensure!(!got.is_empty(), "{name}: no rustc.wasm inside");
                // The builder already strips the blob, so this is usually a no-op.
                let bytes = fs::read(rustc_dir.join("rustc.wasm"))?;
                let (stripped, dropped) = strip_wasm(&bytes)?;
                if dropped > 0 {
                    fs::write(rustc_dir.join("rustc.wasm"), &stripped)?;
                    eprintln!(
                        "  stripped rustc.wasm: dropped {:.1} MB",
                        dropped as f64 / 1e6
                    );
                }
            }
            "wasip1-sysroot.tar.zst" => {
                let asset = fetch_asset(&lock, name, sha)?;
                stage_bundle(&asset, name, "sysroot-wasip1", &public, &rustc_dir, full)?;
            }
            "egui-sysroot.tar.zst" => {
                let asset = fetch_asset(&lock, name, sha)?;
                stage_bundle(&asset, name, "sysroot-egui", &public, &rustc_dir, full)?;
            }
            "std-sysroot.tar.zst" if full => {
                let asset = fetch_asset(&lock, name, sha)?;
                let got = extract_tar_zst(&asset, |p| Some(public.join(p)))?;
                eprintln!("  x86_64 std-sysroot: {} files", got.len());
            }
            // The site loads neither the loose trees nor std-sysroot, so they are not downloaded.
            _ => {}
        }
    }

    write_assets_meta(&rustc_dir)?;
    eprintln!(
        "xtask: done ({})",
        if full { "full" } else { "bundles only" }
    );
    Ok(())
}

/// Stage one sysroot tarball, whose entries are `rustc/<name>/...` and `rustc/<name>.bundle`.
/// The site reads the bundle, `--full` also writes the loose tree.
fn stage_bundle(
    asset: &Path,
    name: &str,
    sysroot: &str,
    public: &Path,
    rustc_dir: &Path,
    full: bool,
) -> Result<()> {
    let bundle = format!("{sysroot}.bundle");
    if full {
        let got = extract_tar_zst(asset, |p| Some(public.join(p)))?;
        ensure!(
            got.iter().any(|p| p.ends_with(&bundle)),
            "{name}: no {bundle} inside"
        );
        eprintln!("  {sysroot}: {} files + bundle", got.len() - 1);
    } else {
        let got = extract_tar_zst(asset, |p| {
            (p.file_name()?.to_str()? == bundle).then(|| rustc_dir.join(&bundle))
        })?;
        ensure!(!got.is_empty(), "{name}: no {bundle} inside");
    }
    Ok(())
}

fn stage_local(src: &Path, public: &Path, full: bool) -> Result<()> {
    eprintln!("xtask: local mode, copying from {}", src.display());
    let rustc_dir = public.join("rustc");
    fs::create_dir_all(&rustc_dir)?;
    for f in ["rustc.wasm", "sysroot-wasip1.bundle", "sysroot-egui.bundle"] {
        let from = src.join("rustc").join(f);
        ensure!(from.exists(), "{} not found", from.display());
        fs::copy(&from, rustc_dir.join(f))
            .with_context(|| format!("copying {}", from.display()))?;
    }
    if full {
        copy_dir(&src.join("rustc"), &rustc_dir)?;
        if src.join("std-sysroot").exists() {
            copy_dir(&src.join("std-sysroot"), &public.join("std-sysroot"))?;
        }
    }
    write_assets_meta(&rustc_dir)?;
    eprintln!("xtask: done (local{})", if full { ", full" } else { "" });
    Ok(())
}
