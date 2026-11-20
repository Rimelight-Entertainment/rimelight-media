use std::path::{Path, PathBuf};

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("the Slint markup compiles");
    copy_ffmpeg_dlls();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=assets/rimelight-media.ico");
        embed_resource::compile("app.rc", embed_resource::NONE).manifest_optional().expect("the icon resource compiles");
    }
}

/// The FFmpeg libraries Rimelight Media uses (no avdevice, no avfilter).
pub const SHIPPED: &[&str] = &["avcodec-", "avformat-", "avutil-", "swresample-", "swscale-"];

/// Puts the FFmpeg DLLs the app uses next to rimelight-media.exe (the profile folder), so the built app starts when run
/// directly and tools/pack.ps1 finds them.
fn copy_ffmpeg_dlls() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let Some(dir) = std::env::var_os("FFMPEG_DIR").map(PathBuf::from) else { return };
    let bin = dir.join("bin");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let Some(target) = out.ancestors().find(|a| a.file_name().is_some_and(|n| n == profile.as_str())) else { return };
    let Ok(entries) = std::fs::read_dir(&bin) else {
        println!("cargo:warning=FFmpeg is missing: run tools/get-ffmpeg.ps1");
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")) && SHIPPED.iter().any(|s| name.starts_with(s)) {
            copy_if_changed(&p, &target.join(e.file_name()));
        }
    }
}

fn copy_if_changed(from: &Path, to: &Path) {
    let same = matches!((std::fs::metadata(from), std::fs::metadata(to)), (Ok(a), Ok(b)) if a.len() == b.len());
    if !same && let Err(e) = std::fs::copy(from, to) {
        println!("cargo:warning=could not copy {}: {e}", from.display());
    }
}
