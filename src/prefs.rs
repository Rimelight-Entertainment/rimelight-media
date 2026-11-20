//! Settings and the recent-files list, saved as small text files in %APPDATA%\Rimelight Media (the rimelight-space pattern of
//! plain text stores). A missing or broken file means defaults.

use std::path::{Path, PathBuf};

pub const MAX_RECENTS: usize = 12;

#[derive(Debug, Clone)]
pub struct Prefs {
    /// 0 light, 1 dark, 2 follow Windows.
    pub theme: i32,
    pub background: bool,
    pub snip_copy: bool,
    pub snip_save: bool,
    pub snip_open: bool,
    pub rec_system_audio: bool,
    pub rec_mic: bool,
    pub rec_fps: i32,
    pub volume: f32,
    pub screenshot_dir: PathBuf,
    pub recording_dir: PathBuf,
    pub voice_dir: PathBuf,
    pub desktop_shortcut: bool,
    /// File types offered to Windows: pictures, video, music.
    pub file_types: [bool; 3],
    /// True when Rimelight Media turned off Windows' own Print Screen snipping (uninstall turns it back on).
    pub printscreen_changed: bool,
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home).join("Rimelight Media")
}

impl Default for Prefs {
    fn default() -> Self {
        let home = home();
        Prefs {
            theme: 2,
            background: false,
            snip_copy: true,
            snip_save: true,
            snip_open: true,
            rec_system_audio: true,
            rec_mic: false,
            rec_fps: 30,
            volume: 1.0,
            // The folders Windows' own tools use.
            screenshot_dir: home.join("Pictures").join("Screenshots"),
            recording_dir: home.join("Videos").join("Captures"),
            voice_dir: home.join("Documents").join("Sound Recordings"),
            desktop_shortcut: true,
            file_types: [true; 3],
            printscreen_changed: false,
        }
    }
}

fn flag(v: &str) -> bool {
    v == "1" || v.eq_ignore_ascii_case("true")
}

impl Prefs {
    pub fn load() -> Prefs {
        let mut p = Prefs::default();
        let Ok(text) = std::fs::read_to_string(data_dir().join("settings.txt")) else { return p };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "theme" => p.theme = v.parse().unwrap_or(2).clamp(0, 2),
                "background" => p.background = flag(v),
                "snip_copy" => p.snip_copy = flag(v),
                "snip_save" => p.snip_save = flag(v),
                "snip_open" => p.snip_open = flag(v),
                "rec_system_audio" => p.rec_system_audio = flag(v),
                "rec_mic" => p.rec_mic = flag(v),
                "rec_fps" => p.rec_fps = if v == "60" { 60 } else { 30 },
                "volume" => p.volume = v.parse().unwrap_or(1.0f32).clamp(0.0, 1.0),
                "screenshot_dir" if !v.is_empty() => p.screenshot_dir = PathBuf::from(v),
                "recording_dir" if !v.is_empty() => p.recording_dir = PathBuf::from(v),
                "voice_dir" if !v.is_empty() => p.voice_dir = PathBuf::from(v),
                "desktop_shortcut" => p.desktop_shortcut = flag(v),
                "printscreen_changed" => p.printscreen_changed = flag(v),
                "file_types" => {
                    let mut it = v.chars().map(|c| c == '1');
                    p.file_types = [it.next().unwrap_or(true), it.next().unwrap_or(true), it.next().unwrap_or(true)];
                }
                _ => {}
            }
        }
        p
    }

    pub fn save(&self) {
        let b = |v: bool| if v { "1" } else { "0" };
        let text = format!(
            "theme={}\nbackground={}\nsnip_copy={}\nsnip_save={}\nsnip_open={}\nrec_system_audio={}\nrec_mic={}\nrec_fps={}\nvolume={}\nscreenshot_dir={}\nrecording_dir={}\nvoice_dir={}\ndesktop_shortcut={}\nprintscreen_changed={}\nfile_types={}{}{}\n",
            self.theme,
            b(self.background),
            b(self.snip_copy),
            b(self.snip_save),
            b(self.snip_open),
            b(self.rec_system_audio),
            b(self.rec_mic),
            self.rec_fps,
            self.volume,
            self.screenshot_dir.display(),
            self.recording_dir.display(),
            self.voice_dir.display(),
            b(self.desktop_shortcut),
            b(self.printscreen_changed),
            b(self.file_types[0]),
            b(self.file_types[1]),
            b(self.file_types[2]),
        );
        write(&data_dir().join("settings.txt"), &text);
    }
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

pub fn load_recents() -> Vec<PathBuf> {
    std::fs::read_to_string(data_dir().join("recents.txt"))
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

pub fn save_recents(list: &[PathBuf]) {
    let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    write(&data_dir().join("recents.txt"), &text);
}

/// Puts `path` first, without duplicates, keeping at most MAX_RECENTS.
pub fn push_recent(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX_RECENTS);
}
