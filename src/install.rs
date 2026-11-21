//! Windows integration around the Velopack install (src/update.rs): the shortcuts (Start menu > Rimelight, Desktop),
//! file types and default-app capabilities, the start-with-Windows entry and Windows' own Print Screen setting. All per
//! user, no admin. Velopack itself adds the Installed apps entry and removes the program files on uninstall.

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
    crate::picture::logo(256).is_some_and(|pic| write_picture_icon(path, pic))
}

fn write_picture_icon(path: &Path, pic: crate::picture::Picture) -> bool {
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

    pub fn register_file_types(exe: &Path, icon: &Path, kinds: [bool; 3]) -> std::io::Result<()> {
        unregister_file_types();
        let exe_s = exe.display().to_string();
        let open = format!("\"{exe_s}\" \"%1\"");
        let icon_s = if icon.exists() { icon.display().to_string() } else { format!("{exe_s},0") };
        // Each kind of file shows the logo in its own colour (pictures light blue, video purple, music red).
        let type_icon = |kind: &str| {
            let p = icon.with_file_name(format!("rimelight-media-{kind}.ico"));
            if p.exists() { p.display().to_string() } else { icon_s.clone() }
        };
        for ((progid, (name, exts)), on) in PROGIDS.iter().zip([("Picture", IMAGE_EXTS), ("Video", VIDEO_EXTS), ("Music", AUDIO_EXTS)]).zip(kinds) {
            if !on {
                continue;
            }
            let base = format!(r"Software\Classes\{progid}");
            set(&base, "", &format!("{name} ({APP_NAME})"))?;
            set(&format!(r"{base}\DefaultIcon"), "", &type_icon(&name.to_lowercase()))?;
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

    /// Windows' own "Use the Print screen key to open screen capture" (on when the value is missing, Windows 11).
    pub fn windows_printscreen_on() -> bool {
        hkcu()
            .open_subkey(r"Control Panel\Keyboard")
            .and_then(|k| k.get_value::<u32, _>("PrintScreenKeyForSnippingEnabled"))
            .map_or(true, |v| v != 0)
    }

    pub fn set_windows_printscreen(on: bool) {
        if let Ok((k, _)) = hkcu().create_subkey(r"Control Panel\Keyboard") {
            let _ = k.set_value("PrintScreenKeyForSnippingEnabled", &u32::from(on));
        }
    }

    /// The ms-screenclip: link Windows starts for Win+Shift+S (and for Print Screen when its own snipping is on). The
    /// Snipping Tool owns it while installed; once it is removed, Windows starts Rimelight Media instead.
    pub fn set_screenclip(exe: Option<&Path>) {
        let base = r"Software\Classes\ms-screenclip";
        match exe {
            Some(exe) => {
                let _ = set(base, "", "URL:Screen snip");
                let _ = set(base, "URL Protocol", "");
                let _ = set(&format!(r"{base}\shell\open\command"), "", &format!("\"{}\" \"%1\"", exe.display()));
            }
            None => {
                let _ = hkcu().delete_subkey_all(base);
            }
        }
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
    pub fn windows_printscreen_on() -> bool {
        false
    }
    pub fn set_windows_printscreen(_: bool) {}
    pub fn register_file_types(_: &Path, _: &Path, _: [bool; 3]) -> std::io::Result<()> {
        Ok(())
    }
    pub fn unregister_file_types() {}
    pub fn file_types_registered() -> bool {
        false
    }
    pub fn set_run_at_login(_: Option<&Path>) {}
    pub fn set_screenclip(_: Option<&Path>) {}
}

pub use reg::{file_types_registered, set_run_at_login, set_screenclip, set_windows_printscreen, windows_printscreen_on};

/// Registers the chosen file types (pictures, video, music) for this exe; the others are removed. An installed copy keeps
/// the same path across updates.
pub fn register_file_types(kinds: [bool; 3]) -> Result<(), String> {
    let icon = icon_path();
    write_icon(&icon);
    for (kind, top, bottom) in [("picture", "#7DD3FC", "#0284C7"), ("video", "#C4B5FD", "#7C3AED"), ("music", "#FDA4AF", "#E11D48")] {
        if let Some(pic) = crate::picture::logo_tinted(256, top, bottom) {
            write_picture_icon(&icon.with_file_name(format!("rimelight-media-{kind}.ico")), pic);
        }
    }
    reg::register_file_types(&current_exe(), &icon, kinds).map_err(|e| e.to_string())
}

/// What shortcuts start: the installer's launcher ("Rimelight Media.exe" in the install folder, always the newest
/// version), else this exe.
pub fn launcher_path() -> PathBuf {
    let exe = current_exe();
    exe.parent()
        .and_then(Path::parent)
        .map(|root| root.join("Rimelight Media.exe"))
        .filter(|p| p.exists())
        .unwrap_or(exe)
}

fn powershell(script: &str) {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = cmd.status();
}

fn quote(p: &Path) -> String {
    p.display().to_string().replace('\'', "''")
}

/// The Start menu folder for every Rimelight app ("Rimelight", like JetBrains' folder).
const START_FOLDER: &str = "$([Environment]::GetFolderPath('Programs'))\\Rimelight";
const DESKTOP: &str = "$([Environment]::GetFolderPath('Desktop'))";

fn shortcut_script(folder: &str) -> String {
    format!(
        "New-Item -ItemType Directory -Force \"{folder}\" | Out-Null; $s=(New-Object -ComObject WScript.Shell).CreateShortcut(\"{folder}\\{APP_NAME}.lnk\"); $s.TargetPath='{}'; $s.WorkingDirectory='{}'; $s.IconLocation='{},0'; $s.Description='Pictures, video, music and screen capture'; $s.Save()",
        quote(&launcher_path()),
        quote(&current_exe().parent().map(Path::to_path_buf).unwrap_or_default()),
        quote(&current_exe()),
    )
}

/// Start menu > Rimelight > Rimelight Media, and the Desktop shortcut when desktop. Also removes the shortcut 0.1.0's
/// installer put at the top of the Start menu.
pub fn set_shortcuts(desktop: bool) {
    let mut script = shortcut_script(START_FOLDER);
    script.push_str("; Remove-Item -Force -ErrorAction SilentlyContinue \"$([Environment]::GetFolderPath('Programs'))\\Rimelight Media.lnk\"");
    if desktop {
        script.push_str("; ");
        script.push_str(&shortcut_script(DESKTOP));
    } else {
        script.push_str(&format!("; Remove-Item -Force -ErrorAction SilentlyContinue \"{DESKTOP}\\{APP_NAME}.lnk\""));
    }
    powershell(&script);
}

/// Removes both shortcuts, and the Rimelight Start menu folder when nothing else is in it.
fn remove_shortcuts() {
    powershell(&format!(
        "Remove-Item -Force -ErrorAction SilentlyContinue \"{START_FOLDER}\\{APP_NAME}.lnk\",\"{DESKTOP}\\{APP_NAME}.lnk\"; if ((Test-Path \"{START_FOLDER}\") -and -not (Get-ChildItem \"{START_FOLDER}\")) {{ Remove-Item -Force \"{START_FOLDER}\" }}"
    ));
}

/// After install: the shortcuts, all file types, start with Windows (so the Print Screen keys work), using the choices
/// saved in the settings (defaults on a first install). The first-run window then lets the person change them.
pub fn after_install() {
    let p = crate::prefs::Prefs::load();
    set_shortcuts(p.desktop_shortcut);
    let _ = register_file_types(p.file_types);
    let mut p = p;
    p.background = true;
    p.save();
    set_run_at_login(Some(&launcher_path()));
    set_screenclip(Some(&launcher_path()));
}

/// After an update: the same integration, kept as the person chose it.
pub fn after_update() {
    let p = crate::prefs::Prefs::load();
    set_shortcuts(p.desktop_shortcut);
    let _ = register_file_types(p.file_types);
    if p.background {
        set_run_at_login(Some(&launcher_path()));
    }
    set_screenclip(Some(&launcher_path()));
}

/// Removes everything this module added, and the app's settings folder (the uninstall hook runs it). Pictures,
/// recordings and voice notes the person made are theirs and stay.
pub fn unregister_all() {
    let p = crate::prefs::Prefs::load();
    if p.printscreen_changed {
        set_windows_printscreen(true);
    }
    remove_shortcuts();
    reg::unregister_file_types();
    set_run_at_login(None);
    set_screenclip(None);
    let _ = std::fs::remove_dir_all(crate::prefs::data_dir());
}

/// Windows Settings > Default apps, on Rimelight Media's page. (Started through the shell: Explorer given an
/// ms-settings link with an encoded space opened the Documents folder instead.)
pub fn open_default_apps_settings() {
    if !crate::platform::open_uri(&format!("ms-settings:defaultapps?registeredAppUser={APP_NAME}")) {
        crate::platform::open_uri("ms-settings:defaultapps");
    }
}

pub fn open_printscreen_settings() {
    crate::platform::open_uri("ms-settings:easeofaccess-keyboard");
}
