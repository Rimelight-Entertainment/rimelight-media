//! Build script for ffmpeg-sys-next with committed bindings (CV-V.2, decision 0046).
//!
//! Upstream runs bindgen (clang) on the FFmpeg headers and a C feature check. Their whole output for one target was
//! recorded once and is checked in under `bindings/`:
//! - `<target>.rs`: the bindings, copied to `OUT_DIR/bindings.rs` (upstream's `src/lib.rs` includes that file);
//! - `<target>.cfg`: every `cargo:` line upstream printed except linking (cfg flags, check-cfg lines and the
//!   `DEP_FFMPEG_*` metadata `ffmpeg-next` reads), printed again here.
//!
//! Linking: the FFmpeg import libraries come from `cargo xtask ffmpeg`, which puts the pinned FFmpeg build in
//! `<target dir>/ffmpeg/current` (or `FFMPEG_DIR` points elsewhere). The DLLs are copied into the profile folder
//! (`target/debug` or `target/release`), next to the app's exe, so it starts when run directly; that folder is also
//! reported as a native search path, because cargo puts search paths inside the profile folder (and only those) on
//! `PATH` for `cargo run` and `cargo test`. Test binaries live in per-unit folders under `build/`, so this is how
//! they find the DLLs.
//! A missing FFmpeg folder is only a warning: `cargo check` and clippy do not link.
//!
//! Recording the files again (new FFmpeg version or a new target): build upstream ffmpeg-sys-next once with
//! `FFMPEG_DIR` and `LIBCLANG_PATH` set, then copy `out/bindings.rs` and the `cargo:` lines of its build output.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// The libraries upstream links, in its order.
const LIBS: &[&str] = &[
    "avcodec",
    "avdevice",
    "avfilter",
    "avformat",
    "avutil",
    "swresample",
    "swscale",
];

fn main() {
    let target = env::var("TARGET").expect("TARGET is set by cargo");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("set by cargo"));
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");

    let bindings = manifest.join("bindings").join(format!("{target}.rs"));
    let cfg = manifest.join("bindings").join(format!("{target}.cfg"));
    if !bindings.exists() || !cfg.exists() {
        panic!(
            "FFmpeg bindings are checked in for x86_64-pc-windows-msvc only; {target} has none. Video plays on \
             Windows first (decision 0046); see this build script's doc comment to record another target."
        );
    }
    println!("cargo:rerun-if-changed={}", bindings.display());
    println!("cargo:rerun-if-changed={}", cfg.display());
    fs::copy(&bindings, out_dir.join("bindings.rs")).expect("copy the bindings to OUT_DIR");
    for line in fs::read_to_string(&cfg).expect("read the recorded build output").lines() {
        if line.starts_with("cargo:") {
            println!("{line}");
        }
    }

    match ffmpeg_dir(&out_dir) {
        // Rimelight Media: the app's own build script copies the DLLs it uses next to the exe.
        Some(dir) => println!("cargo:rustc-link-search=native={}", dir.join("lib").display()),
        None => println!("cargo:warning=FFmpeg is not set up: run tools/get-ffmpeg.ps1 (check and clippy work without it)"),
    }
    // Rimelight Media: only the libraries whose feature is on (ffmpeg-next without `device` and `filter` doesn't need
    // avdevice and avfilter, so they don't ship).
    for lib in LIBS {
        if *lib == "avutil" || env::var_os(format!("CARGO_FEATURE_{}", lib.to_ascii_uppercase())).is_some() {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
}

/// `FFMPEG_DIR`, else the first `ffmpeg/current` folder above `OUT_DIR` (the target folder's root).
fn ffmpeg_dir(out_dir: &Path) -> Option<PathBuf> {
    if let Some(dir) = env::var_os("FFMPEG_DIR") {
        let dir = PathBuf::from(dir);
        return dir.join("lib").is_dir().then_some(dir);
    }
    out_dir
        .ancestors()
        .map(|a| a.join("ffmpeg").join("current"))
        .find(|candidate| candidate.join("lib").is_dir())
}

