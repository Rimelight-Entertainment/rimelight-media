//! Windows integration around the Velopack install (src/update.rs): file types and default-app capabilities, and the
//! optional start-with-Windows entry. All per user, no admin. Velopack itself adds the Start menu shortcut, the
//! Installed apps entry and removes the files on uninstall.

use std::path::{Path, PathBuf};

use crate::files::{AUDIO_EXTS, IMAGE_EXTS, VIDEO_EXTS};

pub const APP_NAME: &str = "Rimelight Media";
const EXE_NAME: &str = "rimelight-media.exe";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const CAPS_KEY: &str = r"Software\RimelightMedia\Capabilities";
const PROGIDS: [&str; 3] = ["RimelightMedia.Image", "RimelightMedia.Video", "RimelightMedia.Audio"];

pub fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from(EXE_NAME))
}

fn icon_path() -> PathBuf {
    crate::prefs::data_dir().join("rimelight-media.ico")
}

/// Writes the app icon (the logo) as an .ico file, for file types and shortcuts.
pub fn write_icon(path: &Path) -> bool {
    let Some(pic) = crate::picture::logo(256) else { return false };
    let Some(img) = image::RgbaImage::from_raw(pic.w, pic.h, pic.rgba) else { return false };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    image::DynamicImage::ImageRgba8(img).save_with_format(path, image::ImageFormat::Ico).is_ok()
}

#[cfg(windows)]
mod reg {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    fn set(path: &str, name: &str, value: &str) -> std::io::Result<()> {
        let (key, _) = hkcu().create_subkey(path)?;
        key.set_value(name, &value)
    }

    pub fn register_file_types(exe: &Path, icon: &Path) -> std::io::Result<()> {
        let exe_s = exe.display().to_string();
        let open = format!("\"{exe_s}\" \"%1\"");
        let icon_s = if icon.exists() { icon.display().to_string() } else { format!("{exe_s},0") };
        for (progid, (name, exts)) in PROGIDS.iter().zip([("Picture", IMAGE_EXTS), ("Video", VIDEO_EXTS), ("Music", AUDIO_EXTS)]) {
            let base = format!(r"Software\Classes\{progid}");
            set(&base, "", &format!("{name} ({APP_NAME})"))?;
            set(&format!(r"{base}\DefaultIcon"), "", &icon_s)?;
            set(&format!(r"{base}\shell\open\command"), "", &open)?;
            for ext in exts {
                set(&format!(r"Software\Classes\.{ext}\OpenWithProgids"), progid, "")?;
                set(&format!(r"{CAPS_KEY}\FileAssociations"), &format!(".{ext}"), progid)?;
                set(&format!(r"Software\Classes\Applications\{EXE_NAME}\SupportedTypes"), &format!(".{ext}"), "")?;
            }
        }
        set(&format!(r"Software\Classes\Applications\{EXE_NAME}\shell\open\command"), "", &open)?;
        set(&format!(r"Software\Classes\Applications\{EXE_NAME}"), "FriendlyAppName", APP_NAME)?;
        set(CAPS_KEY, "ApplicationName", APP_NAME)?;
        set(CAPS_KEY, "ApplicationIcon", &icon_s)?;
        set(CAPS_KEY, "ApplicationDescription", "Pictures, video, music, screenshots and screen recording in one small app.")?;
        set(r"Software\RegisteredApplications", APP_NAME, CAPS_KEY)?;
        crate::platform::file_types_changed();
        Ok(())
    }

    pub fn unregister_file_types() {
        let hk = hkcu();
        for progid in PROGIDS {
            let _ = hk.delete_subkey_all(format!(r"Software\Classes\{progid}"));
        }
        for ext in IMAGE_EXTS.iter().chain(VIDEO_EXTS).chain(AUDIO_EXTS) {
            if let Ok(k) = hk.open_subkey_with_flags(format!(r"Software\Classes\.{ext}\OpenWithProgids"), KEY_ALL_ACCESS) {
                for progid in PROGIDS {
                    let _ = k.delete_value(progid);
                }
            }
        }
        let _ = hk.delete_subkey_all(format!(r"Software\Classes\Applications\{EXE_NAME}"));
        let _ = hk.delete_subkey_all(r"Software\RimelightMedia");
        if let Ok(k) = hk.open_subkey_with_flags(r"Software\RegisteredApplications", KEY_ALL_ACCESS) {
            let _ = k.delete_value(APP_NAME);
        }
        crate::platform::file_types_changed();
    }

    pub fn file_types_registered() -> bool {
        hkcu().open_subkey(format!(r"Software\Classes\{}\shell\open\command", PROGIDS[0])).is_ok()
    }

    pub fn set_run_at_login(exe: Option<&Path>) {
        let Ok((key, _)) = hkcu().create_subkey(RUN_KEY) else { return };
        match exe {
            Some(exe) => {
                let _ = key.set_value(APP_NAME, &format!("\"{}\" --background", exe.display()));
            }
            None => {
                let _ = key.delete_value(APP_NAME);
            }
        }
    }
}

#[cfg(not(windows))]
mod reg {
    use std::path::Path;
    pub fn register_file_types(_: &Path, _: &Path) -> std::io::Result<()> {
        Ok(())
    }
    pub fn unregister_file_types() {}
    pub fn file_types_registered() -> bool {
        false
    }
    pub fn set_run_at_login(_: Option<&Path>) {}
}

pub use reg::{file_types_registered, set_run_at_login};

/// Registers the file types for this exe (an installed copy keeps the same path across updates).
pub fn register_file_types() -> Result<(), String> {
    let icon = icon_path();
    if !icon.exists() {
        write_icon(&icon);
    }
    reg::register_file_types(&current_exe(), &icon).map_err(|e| e.to_string())
}

/// Removes everything this module added, and the app's settings folder (the uninstall hook runs it). Pictures,
/// recordings and voice notes the person made are theirs and stay.
pub fn unregister_all() {
    reg::unregister_file_types();
    set_run_at_login(None);
    let _ = std::fs::remove_dir_all(crate::prefs::data_dir());
}

pub fn open_default_apps_settings() {
    let _ = std::process::Command::new("explorer").arg("ms-settings:defaultapps?registeredAppUser=Rimelight%20Media").spawn();
}

pub fn open_printscreen_settings() {
    let _ = std::process::Command::new("explorer").arg("ms-settings:easeofaccess-keyboard").spawn();
}
