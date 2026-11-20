//! Rimelight Media: one small Windows app for pictures, video, music, screenshots, screen recording and voice recording.
//!
//! Usage:
//! - `rimelight-media [file]`: open a file (how Windows starts it for a default app).
//! - `--background`: start in the tray with the global keys and no window (start-with-Windows entry).
//! - `--snip region|window|screen`: start a snip at once.
//! - `--write-icon <file.ico>`: write the app icon and exit (used by tools/pack.ps1).
//! - `--screenshot <file.png>`: open the window without focus, behind other windows, draw about 120 frames, save the
//!   window's picture and exit (used by automated checks).

// Release builds are GUI apps: no console window (as rimelight-space's desktop app).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod capture;
mod capturectl;
mod files;
mod hotkeys;
mod install;
mod media;
mod mediactl;
mod picture;
mod platform;
mod prefs;
mod settingsctl;
mod setupctl;
mod ui;
mod update;
mod viewer;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use slint::ComponentHandle as _;

const TOOL: &str = "rimelight-media";

struct Args {
    file: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    background: bool,
    snip: Option<String>,
    write_icon: Option<PathBuf>,
}

fn parse() -> Result<Args, String> {
    let mut args = Args { file: None, screenshot: None, background: false, snip: None, write_icon: None };
    let mut it = std::env::args_os().skip(1);
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--screenshot") => args.screenshot = Some(it.next().ok_or("--screenshot needs a file")?.into()),
            Some("--background") => args.background = true,
            Some("--write-icon") => args.write_icon = Some(it.next().ok_or("--write-icon needs a file")?.into()),
            Some("--snip") => args.snip = Some(it.next().and_then(|s| s.into_string().ok()).ok_or("--snip needs region, window or screen")?),
            Some(flag) if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => args.file = Some(PathBuf::from(a)),
        }
    }
    Ok(args)
}

/// Developer check of the engine without the window (set RIMELIGHT_MEDIA_SELFTEST to a folder holding sample.mkv): trims the
/// sample, grabs the screen and records the screen with the computer's sound for two seconds.
fn selftest(dir: &std::path::Path) -> ExitCode {
    let mut ok = true;
    let trimmed = dir.join("selftest-trim.mkv");
    let _ = std::fs::remove_file(&trimmed);
    match media::export::trim(&dir.join("sample.mkv"), &trimmed, 1000, 3000) {
        Ok(()) => match media::player::probe(&trimmed) {
            Ok(i) => println!("trim: ok, {} ms, {} audio, {} subtitle tracks", i.duration_ms, i.audio.len(), i.subtitles.len()),
            Err(e) => {
                ok = false;
                println!("trim: written but unreadable: {e}")
            }
        },
        Err(e) => {
            ok = false;
            println!("trim: failed: {e}")
        }
    }
    match media::player::probe(&dir.join("sample.mkv")).and_then(|info| {
        let p = media::player::Player::open(&dir.join("sample.mkv"), &info, None, 0, (640, 360))?;
        p.set_speed(2.0);
        p.play();
        let t = std::time::Instant::now();
        while t.elapsed() < Duration::from_millis(1500) {
            let _ = p.take_frame();
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(p.position_ms())
    }) {
        // 1.5 s at 2x is about 3 s of video (minus the start-up).
        Ok(ms) if (2_400..=3_200).contains(&ms) => println!("speed: ok, 1.5 s at 2x reached {ms} ms"),
        Ok(ms) => {
            ok = false;
            println!("speed: wrong, 1.5 s at 2x reached {ms} ms")
        }
        Err(e) => {
            ok = false;
            println!("speed: failed: {e}")
        }
    }
    match picture::load_animation(&dir.join("sample.gif")) {
        Some(a) => println!("gif: ok, {} frames, first {} ms", a.frames.len(), a.frames[0].1),
        None => {
            ok = false;
            println!("gif: no frames")
        }
    }
    match capture::grab::screenshot() {
        Ok((pic, rect)) => println!("screenshot: ok, {}x{} at {},{}", pic.w, pic.h, rect.x, rect.y),
        Err(e) => {
            ok = false;
            println!("screenshot: failed: {e}")
        }
    }
    let path = dir.join("selftest-recording.mp4");
    let _ = std::fs::remove_file(&path);
    let rect = platform::ScreenRect { x: 0, y: 0, w: 640, h: 360 };
    match capture::recorder::start(capture::recorder::Options { screen: Some((rect, 30)), system_audio: true, mic: false, path }) {
        Ok(rec) => {
            println!("recording: started {:?}", rec.notes);
            std::thread::sleep(Duration::from_secs(2));
            match rec.finish().and_then(|p| media::player::probe(&p).map(|i| (p, i))) {
                Ok((p, i)) => println!("recording: ok, {} ms, video {:?}, {} audio tracks, {}", i.duration_ms, i.video, i.audio.len(), p.display()),
                Err(e) => {
                    ok = false;
                    println!("recording: failed: {e}")
                }
            }
        }
        Err(e) => {
            ok = false;
            println!("recording: failed to start: {e}")
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn main() -> ExitCode {
    // First: during install, update and uninstall Velopack runs its hooks here and exits.
    update::init();
    let args = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{TOOL}: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(path) = &args.write_icon {
        return if install::write_icon(path) { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    if let Some(dir) = std::env::var_os("RIMELIGHT_MEDIA_SELFTEST") {
        return selftest(&PathBuf::from(dir));
    }

    let checking = args.screenshot.is_some();
    let mut selector = slint::BackendSelector::new().backend_name("winit".into());
    if checking {
        // A check never takes focus and stays behind the person's windows (a covered window still draws on Windows).
        selector = selector.with_winit_window_attributes_hook(|attrs| {
            let attrs = attrs.with_active(false);
            #[cfg(windows)]
            let attrs = attrs.with_window_level(slint::winit_030::winit::window::WindowLevel::AlwaysOnBottom);
            attrs
        });
    }
    if let Err(e) = selector.select() {
        eprintln!("{TOOL}: no window system: {e}");
        return ExitCode::FAILURE;
    }

    let app = match app::build(args.background && !checking) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{TOOL}: {e}");
            return ExitCode::FAILURE;
        }
    };

    if !checking {
        app::start_hotkeys(&app);
        let keep = app.prefs.borrow().background;
        capturectl::ensure_tray(&app, keep);
    }

    if let Some(file) = &args.file {
        app::open_path(&app, file);
    }
    // The installer's first run: the setup window instead of the app.
    let first_run = !checking && update::FIRST_RUN.load(std::sync::atomic::Ordering::Relaxed) && setupctl::show(&app);
    if !(args.background && !checking) && !first_run {
        if let Err(e) = app.ui.show() {
            eprintln!("{TOOL}: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Some(mode) = &args.snip {
        capturectl::snip(&app, mode);
    }
    if !checking {
        app::check_updates(&app, false);
    }

    // The check: about 120 frames (~2 s), then the window's picture.
    let _check_timer = args.screenshot.map(|out| {
        let timer = slint::Timer::default();
        let frames = std::cell::Cell::new(0u32);
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(16), move || {
            app::with_app(|a| {
                a.ui.window().request_redraw();
                frames.set(frames.get() + 1);
                if frames.get() < 120 {
                    return;
                }
                let code = match a.ui.window().take_snapshot() {
                    Ok(buf) => match image::RgbaImage::from_raw(buf.width(), buf.height(), buf.as_bytes().to_vec()) {
                        Some(img) => {
                            if let Some(dir) = out.parent() {
                                let _ = std::fs::create_dir_all(dir);
                            }
                            match img.save_with_format(&out, image::ImageFormat::Png) {
                                Ok(()) => 0,
                                Err(e) => {
                                    eprintln!("{TOOL}: could not save {}: {e}", out.display());
                                    1
                                }
                            }
                        }
                        None => 1,
                    },
                    Err(e) => {
                        eprintln!("{TOOL}: could not take the window's picture: {e}");
                        1
                    }
                };
                if code != 0 {
                    std::process::exit(code);
                }
                let _ = slint::quit_event_loop();
            });
        });
        timer
    });

    let result = slint::run_event_loop_until_quit();
    drop(_check_timer);
    app::shutdown();
    drop(app);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{TOOL}: {e}");
            ExitCode::FAILURE
        }
    }
}
