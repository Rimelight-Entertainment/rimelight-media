//! Settings page: preferences, install and default apps, keys, folders, theme.

use std::rc::Rc;

use slint::ComponentHandle as _;

use crate::app::{App, toast, with_app};
use crate::install;
use crate::ui;

fn settings(app: &Rc<App>) -> ui::Settings<'_> {
    app.ui.global::<ui::Settings>()
}

pub fn wire(app: &Rc<App>) {
    let s = settings(app);
    s.on_changed(|| with_app(save_from_ui));
    s.on_check_updates(|| with_app(|a| crate::app::check_updates(a, true)));
    s.on_open_releases(|| {
        if let Some(url) = crate::update::releases_page() {
            let _ = std::process::Command::new("explorer").arg(url).spawn();
        }
    });
    s.on_register_defaults(|| {
        with_app(|a| match install::register_file_types() {
            Ok(()) => {
                toast(a, "File types registered. Now pick Rimelight Media in Windows' default apps.");
                install::open_default_apps_settings();
                refresh(a);
            }
            Err(e) => toast(a, format!("Could not register the file types: {e}")),
        })
    });
    s.on_open_default_settings(install::open_default_apps_settings);
    s.on_open_printscreen_settings(install::open_printscreen_settings);
    s.on_pick_folder(|screenshots| {
        with_app(move |a| {
            let current = if screenshots { a.prefs.borrow().screenshot_dir.clone() } else { a.prefs.borrow().recording_dir.clone() };
            if let Some(dir) = rfd::FileDialog::new().set_directory(current).pick_folder() {
                {
                    let mut p = a.prefs.borrow_mut();
                    if screenshots {
                        p.screenshot_dir = dir;
                    } else {
                        p.recording_dir = dir;
                    }
                    p.save();
                }
                refresh(a);
            }
        })
    });
    refresh(app);
}

pub fn refresh(app: &Rc<App>) {
    let s = settings(app);
    let p = app.prefs.borrow();
    s.set_theme(p.theme);
    s.set_background(p.background);
    s.set_snip_copy(p.snip_copy);
    s.set_snip_save(p.snip_save);
    s.set_snip_open(p.snip_open);
    s.set_rec_system_audio(p.rec_system_audio);
    s.set_rec_mic(p.rec_mic);
    s.set_rec_fps(p.rec_fps);
    s.set_screenshot_folder(p.screenshot_dir.display().to_string().into());
    s.set_recording_folder(p.recording_dir.display().to_string().into());
    drop(p);
    let version = crate::update::installed_version();
    s.set_installed(version.is_some());
    s.set_install_state(
        match &version {
            Some(v) => format!("Installed, version {v}. Updates come from {}.", crate::update::feed_name()),
            None => format!("Not installed: this is a build from source (version {}). Install it with RimelightMedia-win-Setup.exe from the releases page, or tools/pack.ps1 for a local installer.", env!("CARGO_PKG_VERSION")),
        }
        .into(),
    );
    s.set_defaults_state(
        if install::file_types_registered() {
            "Rimelight Media is registered for pictures, video and music. Pick it as the default in Windows Settings > Apps > Default apps."
        } else {
            "Register Rimelight Media's file types first, then pick it as the default in Windows' default apps."
        }
        .into(),
    );
    let hk = app.hotkeys.borrow().as_ref().map(|h| h.status.clone()).unwrap_or_else(|| "The keys are off (they could not be registered).".into());
    s.set_hotkeys_status(hk.into());
    s.set_about(
        format!(
            "Rimelight Media {}. Built with Rust and Slint. Video and sound by FFmpeg (LGPL), pictures by the image and resvg crates, icons from Lucide (ISC).",
            env!("CARGO_PKG_VERSION")
        )
        .into(),
    );
}

fn save_from_ui(app: &Rc<App>) {
    let s = settings(app);
    let background_changed = {
        let mut p = app.prefs.borrow_mut();
        let before = p.background;
        p.theme = s.get_theme();
        p.background = s.get_background();
        p.snip_copy = s.get_snip_copy();
        p.snip_save = s.get_snip_save();
        p.snip_open = s.get_snip_open();
        p.rec_system_audio = s.get_rec_system_audio();
        p.rec_mic = s.get_rec_mic();
        p.rec_fps = s.get_rec_fps();
        p.save();
        before != p.background
    };
    apply_theme(app);
    if background_changed {
        let on = app.prefs.borrow().background;
        install::set_run_at_login(on.then(install::current_exe).as_deref());
        crate::capturectl::ensure_tray(app, on);
        toast(app, if on { "Rimelight Media now starts with Windows and stays in the tray." } else { "Rimelight Media no longer starts with Windows." });
    }
}

pub fn apply_theme(app: &Rc<App>) {
    let theme = match app.prefs.borrow().theme {
        0 => ui::ThemeChoice::Light,
        1 => ui::ThemeChoice::Dark,
        _ => ui::ThemeChoice::System,
    };
    app.ui.global::<ui::Tokens>().set_theme(theme);
}
