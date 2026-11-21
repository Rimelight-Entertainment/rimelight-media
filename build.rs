use std::path::{Path, PathBuf};

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("the Slint markup compiles");
    copy_ffmpeg_dlls();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=assets/rimelight-media.ico");
        // The version for the VERSIONINFO block (Task Manager shows FileDescription as the app's name).
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
        let mut parts: Vec<String> = version.split(['.', '-']).take(3).map(str::to_string).collect();
        parts.resize(3, "0".into());
        let commas = format!("{},{},{},0", parts[0], parts[1], parts[2]);
        // app.rc is a template: fill in the version and the icon's full path, then compile the copy.
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default().replace('\\', "/");
        let rc = std::fs::read_to_string("app.rc")
            .expect("app.rc")
            .replace("APP_VERSION_COMMAS", &commas)
            .replace("APP_VERSION_TEXT", &format!("\"{version}\""))
            .replace("\"assets/", &format!("\"{manifest}/assets/"));
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("app.rc");
        std::fs::write(&out, rc).expect("write app.rc");
        embed_resource::compile(&out, embed_resource::NONE).manifest_optional().expect("the resources compile");
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
    // A new FFmpeg (tools/get-ffmpeg.ps1) must replace the DLLs next to the exe.
    println!("cargo:rerun-if-changed={}", bin.display());
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
