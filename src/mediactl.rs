//! The player page: open, play, seek, volume, audio and subtitle tracks, repeat, frame snapshots and trimming.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle as _, ModelRc, SharedString, VecModel};

use crate::app::{App, go, post, remember, toast, update_title, with_app};
use crate::files;
use crate::media::player::{self, MediaInfo, Player, SUB_EXTERNAL, SUB_OFF};
use crate::media::{export, subs};
use crate::ui;

#[derive(Clone, PartialEq)]
enum SubChoice {
    Off,
    Stream(usize),
    File(PathBuf),
}

pub struct MediaState {
    pub path: Option<PathBuf>,
    info: Option<MediaInfo>,
    player: Option<Player>,
    audio_track: Option<usize>,
    sub: SubChoice,
    sub_files: Vec<PathBuf>,
    preview: Option<f32>,
    trim: (u64, u64),
    muted: bool,
    /// Repeat: 0 off, 1 once more, 2 forever.
    loop_mode: i32,
    /// Repeat-once has played its extra time.
    repeated: bool,
    last_pos_text: String,
    last_size: (u32, u32),
    generation: u64,
    speed: f32,
    sub_delay_ms: i64,
    sub_scale: f32,
    /// The picture subtitle on screen (by address), so it is only converted once.
    shown_sub_picture: usize,
    /// The subtitle choice before V turned them off.
    sub_before_off: Option<SubChoice>,
}

/// Playback speeds offered (VLC-like steps).
const SPEEDS: [f32; 10] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

fn speed_label(s: f32) -> String {
    let t = format!("{s:.2}");
    format!("{}×", t.trim_end_matches('0').trim_end_matches('.'))
}

impl Default for MediaState {
    fn default() -> Self {
        MediaState {
            path: None,
            info: None,
            player: None,
            audio_track: None,
            sub: SubChoice::Off,
            sub_files: Vec::new(),
            preview: None,
            trim: (0, 0),
            muted: false,
            loop_mode: 0,
            repeated: false,
            last_pos_text: String::new(),
            last_size: (0, 0),
            generation: 0,
            speed: 1.0,
            sub_delay_ms: 0,
            sub_scale: 1.0,
            shown_sub_picture: 0,
            sub_before_off: None,
        }
    }
}

impl MediaState {
    /// Closes the player (and its sound device).
    pub fn take_player(&mut self) {
        self.player = None;
    }
}

fn player_ui(app: &Rc<App>) -> ui::Player<'_> {
    app.ui.global::<ui::Player>()
}

pub fn wire(app: &Rc<App>) {
    let p = player_ui(app);
    p.set_volume(app.prefs.borrow().volume);
    p.on_toggle_play(|| with_app(toggle_play));
    p.on_seek(|f| {
        with_app(move |a| {
            a.media.borrow_mut().preview = None;
            seek_fraction(a, f);
        })
    });
    p.on_seek_preview(|f| {
        with_app(move |a| {
            a.media.borrow_mut().preview = Some(f);
            let dur = a.media.borrow().info.as_ref().map_or(0, |i| i.duration_ms);
            player_ui(a).set_time_text(files::clock((f64::from(f) * dur as f64) as u64).into());
        })
    });
    p.on_skip(|secs| with_app(move |a| skip(a, i64::from(secs) * 1000)));
    p.on_set_volume(|v| {
        with_app(move |a| {
            player_ui(a).set_volume(v);
            a.prefs.borrow_mut().volume = v;
            if let Some(pl) = a.media.borrow().player.as_ref() {
                pl.set_volume(v);
            }
        })
    });
    p.on_toggle_mute(|| with_app(toggle_mute));
    p.on_toggle_loop(|| with_app(cycle_loop));
    p.on_pick_audio(|i| with_app(move |a| pick_audio(a, i as usize)));
    p.on_pick_subtitle(|i| with_app(move |a| pick_subtitle(a, i as usize)));
    p.on_load_subtitle_file(|| with_app(load_subtitle_file));
    p.on_snapshot(|| with_app(snapshot));
    p.on_trim_set(|start| with_app(move |a| trim_set(a, start)));
    p.on_trim_save(|| with_app(trim_save));
    p.on_pick_speed(|i| with_app(move |a| set_speed(a, SPEEDS.get(i as usize).copied().unwrap_or(1.0))));
    p.on_speed_step(|d| with_app(move |a| speed_step(a, d)));
    p.on_sub_delay(|ms| with_app(move |a| sub_delay(a, i64::from(ms))));
    p.on_sub_size(|d| with_app(move |a| sub_size(a, d)));
    refresh_speeds(app);

    app.tick.start(slint::TimerMode::Repeated, Duration::from_millis(8), || with_app(tick));
}

pub fn open(app: &Rc<App>, path: &Path) {
    // Stop what was playing.
    let generation = {
        let mut m = app.media.borrow_mut();
        m.player = None;
        m.generation += 1;
        m.path = Some(path.to_path_buf());
        m.info = None;
        m.generation
    };
    let p = player_ui(app);
    p.set_buffering(true);
    p.set_error(SharedString::new());
    p.set_subtitle(SharedString::new());
    p.set_playing(false);
    p.set_position(0.0);
    p.set_trim_mode(false);
    p.set_frame(slint::Image::default());
    p.set_media_title(files::file_name(path).into());
    p.set_media_detail(SharedString::new());
    p.set_has_video(files::kind_of(path) == Some(files::Kind::Video));
    go(app, if files::kind_of(path) == Some(files::Kind::Audio) { "audio" } else { "video" });
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let result = player::probe(&path);
        post(move |a| {
            if a.media.borrow().generation != generation {
                return;
            }
            match result {
                Ok(info) => start(a, &path, info),
                Err(e) => {
                    let p = player_ui(a);
                    p.set_buffering(false);
                    p.set_error(e.into());
                }
            }
        });
    });
}

fn output_size(app: &Rc<App>) -> (u32, u32) {
    let s = app.ui.window().size();
    (s.width.max(320), s.height.max(240))
}

fn start(app: &Rc<App>, path: &Path, info: MediaInfo) {
    let p = player_ui(app);
    p.set_has_video(info.video.is_some());
    // The extension guessed the tab; the file decides (a .mkv with only sound is music).
    let page = app.ui.global::<ui::Shell>().get_page();
    if matches!(page.as_str(), "audio" | "video") {
        go(app, if info.video.is_some() { "video" } else { "audio" });
    }
    p.set_video_w(info.width.max(1) as i32);
    p.set_video_h(info.height.max(1) as i32);
    p.set_has_sub_picture(false);
    p.set_media_title(info.title.clone().into());
    let mut detail = info.detail.clone();
    if info.video.is_some() {
        detail = format!("{} × {}", info.width, info.height);
    }
    p.set_media_detail(detail.into());
    p.set_length_text(files::clock(info.duration_ms).into());
    p.set_time_text(files::clock(0).into());
    let sub_files = subs::files_beside(path);
    // A default subtitle stream, or a file beside the video, is not turned on by itself (VLC-like: off unless marked
    // as forced/default). Default-marked text streams are turned on.
    let default_sub = info.subtitles.iter().find(|t| t.default && !t.bitmap).map(|t| t.index);
    let size = output_size(app);
    let volume = app.prefs.borrow().volume;
    match Player::open(path, &info, None, 0, size) {
        Ok(pl) => {
            pl.set_volume(volume);
            {
                let m = app.media.borrow();
                pl.set_muted(m.muted);
                pl.set_looping(m.loop_mode == 2);
            }
            {
                let m = app.media.borrow();
                pl.set_speed(m.speed);
                pl.set_sub_delay(m.sub_delay_ms);
            }
            pl.play();
            let mut m = app.media.borrow_mut();
            m.shown_sub_picture = 0;
            m.repeated = false;
            m.audio_track = info.audio.iter().find(|t| t.default).or(info.audio.first()).map(|t| t.index);
            m.trim = (0, info.duration_ms);
            m.sub_files = sub_files;
            m.sub = SubChoice::Off;
            m.last_size = size;
            m.player = Some(pl);
            m.info = Some(info);
        }
        Err(e) => {
            p.set_buffering(false);
            p.set_error(e.into());
            return;
        }
    }
    if let Some(idx) = default_sub {
        set_sub(app, SubChoice::Stream(idx));
    }
    refresh_tracks(app);
    update_trim_text(app);
    update_title(app);
}

fn tick(app: &Rc<App>) {
    let mut m = app.media.borrow_mut();
    let size = output_size(app);
    let Some(pl) = m.player.as_ref() else { return };
    if size != m.last_size {
        pl.set_output_size(size.0, size.1);
    }
    // Repeat once: play it one more time after it ends.
    let replay = m.loop_mode == 1 && !m.repeated && pl.ended();
    if replay {
        pl.play();
    }
    let frame = pl.take_frame();
    let pos = pl.position_ms();
    let dur = pl.duration_ms().max(1);
    let playing = pl.is_playing();
    let buffering = pl.is_buffering() && playing;
    let (subtitle, sub_picture) = pl.subtitle_now();
    let error = pl.error();
    m.last_size = size;
    if replay {
        m.repeated = true;
    }
    let preview = m.preview;
    let text = files::clock(pos);
    let text_changed = text != m.last_pos_text;
    if text_changed {
        m.last_pos_text = text.clone();
    }
    drop(m);

    let p = player_ui(app);
    if let Some(f) = frame {
        let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&f.rgba, f.w, f.h);
        p.set_frame(slint::Image::from_rgba8(buf));
    }
    if preview.is_none() {
        let frac = (pos as f64 / dur as f64) as f32;
        if (p.get_position() - frac).abs() > 0.0005 {
            p.set_position(frac);
        }
        if text_changed {
            p.set_time_text(text.into());
        }
    }
    if p.get_playing() != playing {
        p.set_playing(playing);
    }
    if p.get_buffering() != buffering {
        p.set_buffering(buffering);
    }
    if p.get_subtitle().as_str() != subtitle {
        p.set_subtitle(subtitle.into());
    }
    show_sub_picture(app, sub_picture);
    if let Some(e) = error
        && p.get_error().as_str() != e
    {
        p.set_error(e.into());
    }
}

fn toggle_play(app: &Rc<App>) {
    let m = app.media.borrow();
    if let Some(pl) = m.player.as_ref() {
        if pl.is_playing() { pl.pause() } else { pl.play() }
    }
}

pub fn pause(app: &Rc<App>) {
    if let Some(pl) = app.media.borrow().player.as_ref() {
        pl.pause();
    }
}

/// Leaving the player page: video pauses, music keeps playing.
pub fn leaving(app: &Rc<App>) {
    if player_ui(app).get_has_video() {
        pause(app);
    }
    if app.ui.global::<ui::Shell>().get_full_screen() {
        crate::app::toggle_full_screen(app);
    }
}

fn seek_fraction(app: &Rc<App>, f: f32) {
    let m = app.media.borrow();
    if let Some(pl) = m.player.as_ref() {
        pl.seek((f64::from(f.clamp(0.0, 1.0)) * pl.duration_ms() as f64) as u64);
    }
}

fn skip(app: &Rc<App>, delta_ms: i64) {
    let m = app.media.borrow();
    if let Some(pl) = m.player.as_ref() {
        let to = (pl.position_ms() as i64 + delta_ms).clamp(0, pl.duration_ms() as i64);
        pl.seek(to as u64);
    }
}

fn toggle_mute(app: &Rc<App>) {
    let mut m = app.media.borrow_mut();
    m.muted = !m.muted;
    if let Some(pl) = m.player.as_ref() {
        pl.set_muted(m.muted);
    }
    player_ui(app).set_muted(m.muted);
}

fn change_volume(app: &Rc<App>, delta: f32) {
    let v = (player_ui(app).get_volume() + delta).clamp(0.0, 1.0);
    player_ui(app).invoke_set_volume(v);
    toast(app, format!("Volume {}%", (v * 100.0).round()));
}

fn refresh_tracks(app: &Rc<App>) {
    let m = app.media.borrow();
    let Some(info) = m.info.as_ref() else { return };
    let audio: Vec<ui::Choice> = info
        .audio
        .iter()
        .map(|t| ui::Choice { label: t.label.clone().into(), detail: t.detail.clone().into(), selected: m.audio_track == Some(t.index) })
        .collect();
    let mut subs_list = vec![ui::Choice { label: "Off".into(), detail: SharedString::new(), selected: m.sub == SubChoice::Off }];
    for t in &info.subtitles {
        subs_list.push(ui::Choice { label: t.label.clone().into(), detail: t.detail.clone().into(), selected: m.sub == SubChoice::Stream(t.index) });
    }
    for f in &m.sub_files {
        subs_list.push(ui::Choice { label: files::file_name(f).into(), detail: "File beside the video".into(), selected: m.sub == SubChoice::File(f.clone()) });
    }
    if let SubChoice::File(f) = &m.sub
        && !m.sub_files.contains(f)
    {
        subs_list.push(ui::Choice { label: files::file_name(f).into(), detail: "Subtitle file".into(), selected: true });
    }
    subs_list.push(ui::Choice { label: "Add a subtitle file…".into(), detail: "SRT, VTT, ASS or SSA".into(), selected: false });
    let p = player_ui(app);
    p.set_audio_tracks(ModelRc::new(VecModel::from(audio)));
    p.set_subtitle_tracks(ModelRc::new(VecModel::from(subs_list)));
}

fn pick_audio(app: &Rc<App>, i: usize) {
    let (path, info, pos, playing, idx) = {
        let m = app.media.borrow();
        let (Some(path), Some(info), Some(pl)) = (m.path.clone(), m.info.clone(), m.player.as_ref()) else { return };
        let Some(t) = info.audio.get(i) else { return };
        (path, info.clone(), pl.position_ms(), pl.is_playing(), t.index)
    };
    let size = output_size(app);
    // A new player on the other track, from the same spot.
    app.media.borrow_mut().player = None;
    match Player::open(&path, &info, Some(idx), pos, size) {
        Ok(pl) => {
            let mut m = app.media.borrow_mut();
            pl.set_volume(app.prefs.borrow().volume);
            pl.set_muted(m.muted);
            pl.set_looping(m.loop_mode == 2);
            pl.set_speed(m.speed);
            pl.set_sub_delay(m.sub_delay_ms);
            if playing {
                pl.play();
            }
            m.audio_track = Some(idx);
            m.player = Some(pl);
            let sub = m.sub.clone();
            drop(m);
            set_sub(app, sub);
        }
        Err(e) => player_ui(app).set_error(e.into()),
    }
    refresh_tracks(app);
}

fn set_sub(app: &Rc<App>, choice: SubChoice) {
    let mut m = app.media.borrow_mut();
    let Some(pl) = m.player.as_ref() else { return };
    match &choice {
        SubChoice::Off => pl.set_subtitles(SUB_OFF, Vec::new()),
        SubChoice::Stream(idx) => pl.set_subtitles(*idx as i64, Vec::new()),
        SubChoice::File(f) => match subs::load_file(f) {
            Ok(cues) => pl.set_subtitles(SUB_EXTERNAL, cues),
            Err(e) => {
                drop(m);
                toast(app, format!("Could not read the subtitles: {e}"));
                return;
            }
        },
    }
    m.sub = choice;
}

fn pick_subtitle(app: &Rc<App>, i: usize) {
    let choice = {
        let m = app.media.borrow();
        let Some(info) = m.info.as_ref() else { return };
        let n_streams = info.subtitles.len();
        if i == 0 {
            Some(SubChoice::Off)
        } else if i <= n_streams {
            Some(SubChoice::Stream(info.subtitles[i - 1].index))
        } else if i - 1 - n_streams < m.sub_files.len() {
            Some(SubChoice::File(m.sub_files[i - 1 - n_streams].clone()))
        } else if matches!(&m.sub, SubChoice::File(f) if !m.sub_files.contains(f)) && i == 1 + n_streams + m.sub_files.len() {
            None
        } else {
            drop(m);
            load_subtitle_file(app);
            return;
        }
    };
    if let Some(c) = choice {
        set_sub(app, c);
    }
    refresh_tracks(app);
}

fn load_subtitle_file(app: &Rc<App>) {
    let dir = app.media.borrow().path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf));
    let mut d = rfd::FileDialog::new().set_title("Add subtitles").add_filter("Subtitles", files::SUBTITLE_EXTS);
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    if let Some(f) = d.pick_file() {
        set_sub(app, SubChoice::File(f));
        refresh_tracks(app);
    }
}

fn snapshot(app: &Rc<App>) {
    let (path, pos) = {
        let m = app.media.borrow();
        let (Some(path), Some(pl)) = (m.path.clone(), m.player.as_ref()) else { return };
        (path, pl.position_ms())
    };
    let dir = app.prefs.borrow().screenshot_dir.clone();
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Video").to_string();
    std::thread::spawn(move || {
        let result = player::frame_at(&path, pos).and_then(|pic| {
            let out = files::free_path(&dir, &format!("{stem} {}", files::clock(pos).replace(':', "-")), "png");
            crate::picture::save(&pic, &out).map(|()| out)
        });
        post(move |a| match result {
            Ok(out) => toast(a, format!("Frame saved: {}", files::file_name(&out))),
            Err(e) => toast(a, format!("Could not save the frame: {e}")),
        });
    });
}

fn update_trim_text(app: &Rc<App>) {
    let m = app.media.borrow();
    let dur = m.info.as_ref().map_or(1, |i| i.duration_ms.max(1));
    let (s, e) = m.trim;
    let p = player_ui(app);
    p.set_trim_start((s as f64 / dur as f64) as f32);
    p.set_trim_end((e as f64 / dur as f64) as f32);
    p.set_trim_text(format!("Keep {} to {}  ({})", files::clock(s), files::clock(e), files::clock(e.saturating_sub(s))).into());
}

fn trim_set(app: &Rc<App>, start: bool) {
    {
        let mut m = app.media.borrow_mut();
        let Some(pos) = m.player.as_ref().map(Player::position_ms) else { return };
        let dur = m.info.as_ref().map_or(0, |i| i.duration_ms);
        if start {
            m.trim.0 = pos.min(m.trim.1.saturating_sub(100));
        } else {
            m.trim.1 = pos.max(m.trim.0 + 100).min(dur);
        }
    }
    update_trim_text(app);
}

fn trim_save(app: &Rc<App>) {
    let (path, (s, e)) = {
        let m = app.media.borrow();
        let Some(path) = m.path.clone() else { return };
        (path, m.trim)
    };
    let ext = files::ext_of(&path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Clip").to_string();
    let suggested = format!("{stem} (trimmed).{ext}");
    let mut d = rfd::FileDialog::new().set_title("Save trimmed copy").set_file_name(&suggested).add_filter(&ext.to_uppercase(), &[ext.as_str()]);
    if let Some(dir) = path.parent() {
        d = d.set_directory(dir);
    }
    let Some(out) = d.save_file() else { return };
    if out == path {
        toast(app, "Pick another name: the trimmed copy can't replace the file that is open.");
        return;
    }
    player_ui(app).set_exporting(true);
    std::thread::spawn(move || {
        let result = export::trim(&path, &out, s, e);
        post(move |a| {
            player_ui(a).set_exporting(false);
            match result {
                Ok(()) => {
                    player_ui(a).set_trim_mode(false);
                    toast(a, format!("Saved {}", files::file_name(&out)));
                    remember(a, &out);
                }
                Err(err) => toast(a, format!("Could not trim: {err}")),
            }
        });
    });
}

pub fn on_key(app: &Rc<App>, text: &str, ctrl: bool, shift: bool) -> bool {
    use slint::platform::Key;
    let k = |key: Key| text == SharedString::from(key).as_str();
    if ctrl {
        return false;
    }
    let step = if shift { 30_000 } else { 5_000 };
    if text == " " || text == "k" {
        toggle_play(app);
    } else if k(Key::LeftArrow) {
        skip(app, -step);
    } else if k(Key::RightArrow) {
        skip(app, step);
    } else if k(Key::UpArrow) {
        change_volume(app, 0.05);
    } else if k(Key::DownArrow) {
        change_volume(app, -0.05);
    } else if text == "[" || text == "-" {
        speed_step(app, -1);
    } else if text == "]" || text == "+" {
        speed_step(app, 1);
    } else if text == "=" {
        set_speed(app, 1.0);
    } else if text == "g" {
        sub_delay(app, -100);
    } else if text == "h" {
        sub_delay(app, 100);
    } else if text == "G" {
        sub_size(app, -1);
    } else if text == "H" {
        sub_size(app, 1);
    } else if text == "v" {
        toggle_subs(app);
    } else if text == "m" {
        toggle_mute(app);
    } else if text == "f" {
        crate::app::toggle_full_screen(app);
    } else if text == "l" {
        player_ui(app).invoke_toggle_loop();
    } else if k(Key::Home) {
        seek_fraction(app, 0.0);
    } else {
        return false;
    }
    true
}

fn refresh_speeds(app: &Rc<App>) {
    let current = app.media.borrow().speed;
    let list: Vec<ui::Choice> = SPEEDS
        .iter()
        .map(|s| ui::Choice {
            label: speed_label(*s).into(),
            detail: if (*s - 1.0).abs() < 0.001 { "Normal".into() } else { SharedString::new() },
            selected: (*s - current).abs() < 0.001,
        })
        .collect();
    let p = player_ui(app);
    p.set_speeds(ModelRc::new(VecModel::from(list)));
    p.set_speed_text(speed_label(current).into());
}

fn set_speed(app: &Rc<App>, speed: f32) {
    {
        let mut m = app.media.borrow_mut();
        m.speed = speed;
        if let Some(pl) = m.player.as_ref() {
            pl.set_speed(speed);
        }
    }
    refresh_speeds(app);
    toast(app, format!("Speed {}", speed_label(speed)));
}

fn speed_step(app: &Rc<App>, delta: i32) {
    let current = app.media.borrow().speed;
    let i = SPEEDS.iter().position(|s| (*s - current).abs() < 0.001).unwrap_or(3) as i32;
    let next = (i + delta).clamp(0, SPEEDS.len() as i32 - 1) as usize;
    set_speed(app, SPEEDS[next]);
}

fn sub_delay(app: &Rc<App>, change_ms: i64) {
    let ms = {
        let mut m = app.media.borrow_mut();
        m.sub_delay_ms = if change_ms == 0 { 0 } else { (m.sub_delay_ms + change_ms).clamp(-600_000, 600_000) };
        if let Some(pl) = m.player.as_ref() {
            pl.set_sub_delay(m.sub_delay_ms);
        }
        m.sub_delay_ms
    };
    let text = if ms == 0 { "0 ms".to_string() } else { format!("{ms:+} ms") };
    player_ui(app).set_sub_delay_text(text.clone().into());
    toast(app, format!("Subtitle delay {text}"));
}

fn sub_size(app: &Rc<App>, delta: i32) {
    let scale = {
        let mut m = app.media.borrow_mut();
        m.sub_scale = if delta == 0 { 1.0 } else { (m.sub_scale + 0.1 * delta as f32).clamp(0.5, 3.0) };
        m.sub_scale
    };
    player_ui(app).set_sub_scale(scale);
    toast(app, format!("Subtitle size {}%", (scale * 100.0).round()));
}

/// V: subtitles off, and back to the last choice.
fn toggle_subs(app: &Rc<App>) {
    let (current, before) = {
        let m = app.media.borrow();
        (m.sub.clone(), m.sub_before_off.clone())
    };
    if current != SubChoice::Off {
        app.media.borrow_mut().sub_before_off = Some(current);
        set_sub(app, SubChoice::Off);
        toast(app, "Subtitles off");
    } else if let Some(b) = before {
        set_sub(app, b);
        toast(app, "Subtitles on");
    } else {
        // Nothing chosen yet: the first text track, else a file beside the video.
        let first = {
            let m = app.media.borrow();
            let stream = m.info.as_ref().and_then(|i| i.subtitles.first().map(|t| SubChoice::Stream(t.index)));
            stream.or_else(|| m.sub_files.first().map(|f| SubChoice::File(f.clone())))
        };
        match first {
            Some(c) => {
                set_sub(app, c);
                toast(app, "Subtitles on");
            }
            None => toast(app, "This video has no subtitles. Add a file from the subtitles menu."),
        }
    }
    refresh_tracks(app);
}

fn show_sub_picture(app: &Rc<App>, picture: Option<std::sync::Arc<crate::media::bitmap_sub::SubPicture>>) {
    let p = player_ui(app);
    let key = picture.as_ref().map_or(0, |pic| std::sync::Arc::as_ptr(pic) as usize);
    if app.media.borrow().shown_sub_picture == key {
        return;
    }
    app.media.borrow_mut().shown_sub_picture = key;
    let Some(pic) = picture else {
        p.set_has_sub_picture(false);
        return;
    };
    let (vw, vh) = (p.get_video_w().max(1) as f32, p.get_video_h().max(1) as f32);
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&pic.rgba, pic.w, pic.h);
    p.set_sub_picture(slint::Image::from_rgba8(buf));
    p.set_sub_x(pic.x as f32 / vw);
    p.set_sub_y(pic.y as f32 / vh);
    p.set_sub_w(pic.w as f32 / vw);
    p.set_sub_h(pic.h as f32 / vh);
    p.set_has_sub_picture(true);
}
/// Repeat: off, then once more, then forever.
fn cycle_loop(app: &Rc<App>) {
    let mode = {
        let mut m = app.media.borrow_mut();
        m.loop_mode = (m.loop_mode + 1) % 3;
        m.repeated = false;
        if let Some(pl) = m.player.as_ref() {
            pl.set_looping(m.loop_mode == 2);
        }
        m.loop_mode
    };
    player_ui(app).set_loop_mode(mode);
    toast(app, ["Repeat off", "Repeat once", "Repeat forever"][mode as usize]);
}

/// Empties the Audio/Video tab: stops playback and frees the player and its pictures.
pub fn close(app: &Rc<App>) {
    {
        let mut m = app.media.borrow_mut();
        m.player = None;
        m.generation += 1;
        m.path = None;
        m.info = None;
        m.sub_files.clear();
        m.shown_sub_picture = 0;
        m.repeated = false;
    }
    let p = player_ui(app);
    p.set_frame(slint::Image::default());
    p.set_media_title(SharedString::new());
    p.set_media_detail(SharedString::new());
    p.set_has_video(false);
    p.set_playing(false);
    p.set_position(0.0);
    p.set_subtitle(SharedString::new());
    p.set_has_sub_picture(false);
    p.set_sub_picture(slint::Image::default());
    p.set_error(SharedString::new());
    p.set_buffering(false);
    p.set_trim_mode(false);
    p.set_audio_tracks(ModelRc::default());
    p.set_subtitle_tracks(ModelRc::default());
    crate::app::update_title(app);
}