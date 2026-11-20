//! Snips, screen recordings and voice recordings, with the snip overlay, the recording bar and the tray icon.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle as _, ModelRc, SharedString, VecModel};

use crate::app::{App, go, open_path, post, remember, show_main, toast, with_app};
use crate::capture::{grab, recorder};
use crate::files;
use crate::picture::{self, Picture};
use crate::platform::{self, ScreenRect, TopWindow};
use crate::ui;

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    Snip,
    Record,
}

struct Snip {
    window: ui::SnipWindow,
    shot: Picture,
    rect: ScreenRect,
    windows: Vec<TopWindow>,
    purpose: Purpose,
    main_was_visible: bool,
}

#[derive(Default)]
pub struct CaptureState {
    snip: Option<Snip>,
    busy: bool,
    screen: Option<recorder::Recording>,
    bar: Option<ui::RecBar>,
    voice: Option<recorder::Recording>,
}

pub fn wire(app: &Rc<App>) {
    let shell = app.ui.global::<ui::Shell>();
    shell.on_snip(|mode| with_app(move |a| begin(a, &mode, Purpose::Snip)));
    shell.on_record_screen(|mode| with_app(move |a| begin(a, &mode, Purpose::Record)));
    shell.on_stop_recording(|| with_app(stop_screen));
    let voice = app.ui.global::<ui::Voice>();
    voice.set_folder(app.prefs.borrow().voice_dir.display().to_string().into());
    voice.on_toggle(|| with_app(toggle_voice));
    voice.on_open_take(|i| {
        with_app(move |a| {
            if let Some(p) = takes(a).get(i as usize) {
                open_path(a, p);
            }
        })
    });
    voice.on_open_folder(|| {
        with_app(|a| {
            let dir = a.prefs.borrow().voice_dir.clone();
            let _ = std::fs::create_dir_all(&dir);
            files::reveal(&dir);
        })
    });
    app.slow_tick.start(slint::TimerMode::Repeated, Duration::from_millis(100), || with_app(slow_tick));
}

/// The hotkey/tray entry points.
pub fn hotkey(app: &Rc<App>, action: crate::hotkeys::Action) {
    use crate::hotkeys::Action;
    match action {
        Action::SnipRegion => begin(app, "region", Purpose::Snip),
        Action::SnipWindow => begin(app, "window", Purpose::Snip),
        Action::SnipScreen => begin(app, "screen", Purpose::Snip),
        Action::ToggleRecording => {
            if app.cap.borrow().screen.is_some() {
                stop_screen(app);
            } else {
                begin(app, "region", Purpose::Record);
            }
        }
    }
}

pub fn snip(app: &Rc<App>, mode: &str) {
    begin(app, mode, Purpose::Snip);
}

pub fn record_toggle(app: &Rc<App>) {
    if app.cap.borrow().screen.is_some() { stop_screen(app) } else { begin(app, "region", Purpose::Record) }
}

/// Hides Rimelight Media, grabs every monitor, then shows the overlay (or, for "whole screen" snips, finishes at once).
fn begin(app: &Rc<App>, mode: &str, purpose: Purpose) {
    {
        let mut c = app.cap.borrow_mut();
        if c.busy || c.snip.is_some() {
            return;
        }
        if purpose == Purpose::Record && c.screen.is_some() {
            return;
        }
        c.busy = true;
    }
    let main_was_visible = app.ui.window().is_visible();
    if main_was_visible {
        let _ = app.ui.hide();
    }
    let mode = mode.to_string();
    std::thread::spawn(move || {
        // Let the window fade out of the picture.
        std::thread::sleep(Duration::from_millis(if main_was_visible { 300 } else { 60 }));
        let result = grab::screenshot();
        let windows = platform::top_windows();
        post(move |a| {
            a.cap.borrow_mut().busy = false;
            match result {
                Ok((shot, rect)) => {
                    if mode == "screen" && purpose == Purpose::Snip {
                        finish_snip(a, shot, main_was_visible);
                    } else {
                        show_overlay(a, shot, rect, windows, &mode, purpose, main_was_visible);
                    }
                }
                Err(e) => {
                    if main_was_visible {
                        show_main(a);
                    }
                    toast(a, e);
                }
            }
        });
    });
}

fn show_overlay(app: &Rc<App>, shot: Picture, rect: ScreenRect, windows: Vec<TopWindow>, mode: &str, purpose: Purpose, main_was_visible: bool) {
    let Ok(w) = ui::SnipWindow::new() else { return };
    w.set_shot(picture::to_slint(&shot));
    w.set_shot_w(shot.w as i32);
    w.set_shot_h(shot.h as i32);
    w.set_mode(mode.into());
    w.set_purpose(if purpose == Purpose::Record { "record" } else { "snip" }.into());
    let (fw, fh) = (rect.w.max(1) as f32, rect.h.max(1) as f32);
    let boxes: Vec<ui::WindowBox> = windows
        .iter()
        .map(|t| ui::WindowBox {
            title: t.title.clone().into(),
            x: (t.rect.x - rect.x) as f32 / fw,
            y: (t.rect.y - rect.y) as f32 / fh,
            w: t.rect.w as f32 / fw,
            h: t.rect.h as f32 / fh,
        })
        .collect();
    w.set_windows(ModelRc::new(VecModel::from(boxes.clone())));
    w.on_hit(move |fx, fy| {
        boxes
            .iter()
            .position(|b| fx >= b.x && fx <= b.x + b.w && fy >= b.y && fy <= b.y + b.h)
            .map_or(-1, |i| i as i32)
    });
    w.on_done(|x, y, ww, hh| with_app(move |a| overlay_done(a, x, y, ww, hh)));
    w.on_cancel(|| with_app(overlay_cancel));
    w.window().set_position(slint::PhysicalPosition::new(rect.x, rect.y));
    w.window().set_size(slint::PhysicalSize::new(rect.w as u32, rect.h as u32));
    let _ = w.show();
    // Again after showing: winit may have moved it to fit a monitor.
    w.window().set_position(slint::PhysicalPosition::new(rect.x, rect.y));
    w.window().set_size(slint::PhysicalSize::new(rect.w as u32, rect.h as u32));
    use slint::winit_030::WinitWindowAccessor as _;
    w.window().with_winit_window(|ww| ww.focus_window());
    app.cap.borrow_mut().snip = Some(Snip { window: w, shot, rect, windows, purpose, main_was_visible });
}

fn overlay_cancel(app: &Rc<App>) {
    let snip = app.cap.borrow_mut().snip.take();
    if let Some(s) = snip {
        let _ = s.window.hide();
        if s.main_was_visible {
            show_main(app);
        }
    }
}

fn overlay_done(app: &Rc<App>, x: f32, y: f32, w: f32, h: f32) {
    let Some(s) = app.cap.borrow_mut().snip.take() else { return };
    let _ = s.window.hide();
    let mode = s.window.get_mode();
    // In window mode, use the window's exact pixels.
    let px = |f: f32, total: i32| (f * total as f32).round() as i32;
    let mut area = ScreenRect { x: px(x, s.rect.w), y: px(y, s.rect.h), w: px(w, s.rect.w), h: px(h, s.rect.h) };
    if mode == "window"
        && let Some(t) = s.windows.iter().find(|t| (t.rect.x - s.rect.x - area.x).abs() <= 2 && (t.rect.y - s.rect.y - area.y).abs() <= 2)
    {
        area = ScreenRect { x: t.rect.x - s.rect.x, y: t.rect.y - s.rect.y, w: t.rect.w, h: t.rect.h };
    }
    // Clip to the screenshot.
    let x0 = area.x.clamp(0, s.shot.w as i32 - 1);
    let y0 = area.y.clamp(0, s.shot.h as i32 - 1);
    let x1 = (area.x + area.w).clamp(x0 + 1, s.shot.w as i32);
    let y1 = (area.y + area.h).clamp(y0 + 1, s.shot.h as i32);
    match s.purpose {
        Purpose::Snip => {
            let pic = picture::crop_px(&s.shot, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32);
            finish_snip(app, pic, s.main_was_visible);
        }
        Purpose::Record => {
            let rect = ScreenRect { x: s.rect.x + x0, y: s.rect.y + y0, w: x1 - x0, h: y1 - y0 };
            start_screen(app, rect, s.main_was_visible);
        }
    }
}

fn finish_snip(app: &Rc<App>, pic: Picture, main_was_visible: bool) {
    let (copy, save, open, dir) = {
        let p = app.prefs.borrow();
        (p.snip_copy, p.snip_save, p.snip_open, p.screenshot_dir.clone())
    };
    let mut notes = Vec::new();
    if copy {
        match picture::copy_to_clipboard(&pic) {
            Ok(()) => notes.push("copied".to_string()),
            Err(e) => notes.push(format!("not copied ({e})")),
        }
    }
    let mut saved: Option<PathBuf> = None;
    if save {
        let path = files::free_path(&dir, &files::stamp("Screenshot"), "png");
        match picture::save(&pic, &path) {
            Ok(()) => {
                notes.push(format!("saved as {}", files::file_name(&path)));
                saved = Some(path);
            }
            Err(e) => notes.push(format!("not saved ({e})")),
        }
    }
    if open || main_was_visible {
        crate::viewer::open_picture(app, pic, saved);
        show_main(app);
    }
    if !notes.is_empty() {
        let mut text = notes.join(", ");
        if let Some(c) = text.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        toast(app, format!("Snip {text}"));
    }
}

fn start_screen(app: &Rc<App>, rect: ScreenRect, main_was_visible: bool) {
    let (system_audio, mic, fps, dir) = {
        let p = app.prefs.borrow();
        (p.rec_system_audio, p.rec_mic, p.rec_fps, p.recording_dir.clone())
    };
    let path = recorder::new_path(&dir, "Recording", "mp4");
    let opts = recorder::Options { screen: Some((rect, fps)), system_audio, mic, path };
    match recorder::start(opts) {
        Ok(rec) => {
            let notes = rec.notes.join("; ");
            app.cap.borrow_mut().screen = Some(rec);
            show_bar(app, rect);
            set_recording_flag(app, true);
            if !notes.is_empty() {
                toast(app, notes);
            }
            if main_was_visible {
                // Stay out of the way: the bar has the Stop button.
                let _ = app.ui.window().with_winit_window_minimise();
            }
        }
        Err(e) => {
            show_main(app);
            toast(app, e);
        }
    }
}

trait Minimise {
    fn with_winit_window_minimise(&self) -> Option<()>;
}

impl Minimise for slint::Window {
    fn with_winit_window_minimise(&self) -> Option<()> {
        use slint::winit_030::WinitWindowAccessor as _;
        let _ = self.show();
        self.with_winit_window(|w| w.set_minimized(true))
    }
}

fn show_bar(app: &Rc<App>, rect: ScreenRect) {
    let Ok(bar) = ui::RecBar::new() else { return };
    bar.set_what(format!("{} × {}", rect.w, rect.h).into());
    bar.on_stop(|| with_app(stop_screen));
    bar.on_discard(|| with_app(discard_screen));
    bar.on_drag_by(|dx, dy| {
        with_app(move |a| {
            if let Some(b) = a.cap.borrow().bar.as_ref() {
                let pos = b.window().position();
                let sf = b.window().scale_factor();
                b.window().set_position(slint::PhysicalPosition::new(pos.x + (dx * sf) as i32, pos.y + (dy * sf) as i32));
            }
        })
    });
    let vs = platform::virtual_screen();
    let _ = bar.show();
    let sf = bar.window().scale_factor();
    let bw = (300.0 * sf) as i32;
    // Above the recorded area if there's room, else at the top of the screen. It never shows in the recording.
    let y = if rect.y - vs.y > (60.0 * sf) as i32 { rect.y - (52.0 * sf) as i32 } else { vs.y + (8.0 * sf) as i32 };
    bar.window().set_position(slint::PhysicalPosition::new(rect.x + rect.w / 2 - bw / 2, y));
    use slint::winit_030::WinitWindowAccessor as _;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    bar.window().with_winit_window(|w| {
        if let Ok(h) = w.window_handle()
            && let RawWindowHandle::Win32(h) = h.as_raw()
        {
            platform::exclude_from_capture(h.hwnd.get());
        }
    });
    app.cap.borrow_mut().bar = Some(bar);
}

fn set_recording_flag(app: &Rc<App>, on: bool) {
    app.ui.global::<ui::Shell>().set_recording(on);
    if let Some(t) = app.tray.borrow().as_ref() {
        t.set_recording(on);
    }
}

fn close_bar(app: &Rc<App>) {
    let bar = app.cap.borrow_mut().bar.take();
    if let Some(b) = bar {
        let _ = b.hide();
    }
    set_recording_flag(app, false);
}

pub fn stop_screen(app: &Rc<App>) {
    let Some(rec) = app.cap.borrow_mut().screen.take() else { return };
    close_bar(app);
    toast(app, "Finishing the recording…");
    std::thread::spawn(move || {
        let result = rec.finish();
        post(move |a| match result {
            Ok(path) => {
                show_main(a);
                open_path(a, &path);
                toast(a, format!("Saved {}", files::file_name(&path)));
            }
            Err(e) => {
                show_main(a);
                toast(a, format!("The recording failed: {e}"));
            }
        });
    });
}

fn discard_screen(app: &Rc<App>) {
    let Some(rec) = app.cap.borrow_mut().screen.take() else { return };
    close_bar(app);
    std::thread::spawn(move || rec.discard());
    show_main(app);
    toast(app, "Recording discarded");
}

/// Quitting: finish anything that is recording, so no file is left broken.
pub fn stop_all_now(app: &Rc<App>) {
    let (screen, voice) = {
        let mut c = app.cap.borrow_mut();
        (c.screen.take(), c.voice.take())
    };
    if let Some(r) = screen {
        let _ = r.finish();
    }
    if let Some(r) = voice {
        let _ = r.finish();
    }
}

fn toggle_voice(app: &Rc<App>) {
    let running = app.cap.borrow_mut().voice.take();
    let v = app.ui.global::<ui::Voice>();
    if let Some(rec) = running {
        v.set_recording(false);
        v.set_level(0.0);
        std::thread::spawn(move || {
            let result = rec.finish();
            post(move |a| {
                match result {
                    Ok(path) => {
                        remember(a, &path);
                        toast(a, format!("Saved {}", files::file_name(&path)));
                    }
                    Err(e) => toast(a, format!("The recording failed: {e}")),
                }
                refresh_takes(a);
            });
        });
        return;
    }
    let dir = app.prefs.borrow().voice_dir.clone();
    let opts = recorder::Options { screen: None, system_audio: false, mic: true, path: recorder::new_path(&dir, "Recording", "m4a") };
    match recorder::start(opts) {
        Ok(rec) => {
            app.cap.borrow_mut().voice = Some(rec);
            v.set_recording(true);
        }
        Err(e) => toast(app, e),
    }
}

fn takes(app: &Rc<App>) -> Vec<PathBuf> {
    let dir = app.prefs.borrow().voice_dir.clone();
    let mut list: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| files::kind_of(p) == Some(files::Kind::Audio))
                .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
                .collect()
        })
        .unwrap_or_default();
    list.sort_by(|a, b| b.0.cmp(&a.0));
    list.into_iter().map(|(_, p)| p).collect()
}

pub fn refresh_takes(app: &Rc<App>) {
    let items: Vec<ui::RecentItem> = takes(app)
        .iter()
        .map(|p| ui::RecentItem {
            name: files::file_name(p).into(),
            detail: std::fs::metadata(p).map(|m| files::human_size(m.len())).unwrap_or_default().into(),
            kind: "audio".into(),
        })
        .collect();
    let v = app.ui.global::<ui::Voice>();
    v.set_takes(ModelRc::new(VecModel::from(items)));
    v.set_folder(app.prefs.borrow().voice_dir.display().to_string().into());
}

fn slow_tick(app: &Rc<App>) {
    let c = app.cap.borrow();
    if let Some(r) = c.screen.as_ref() {
        let t: SharedString = files::clock(r.elapsed_ms()).into();
        if let Some(b) = c.bar.as_ref() {
            b.set_time_text(t.clone());
        }
        app.ui.global::<ui::Shell>().set_recording_time(t);
    }
    if let Some(r) = c.voice.as_ref() {
        let v = app.ui.global::<ui::Voice>();
        v.set_time_text(files::clock(r.elapsed_ms()).into());
        v.set_level(r.level());
    }
}

/// The tray icon, while "keep running" is on.
pub fn ensure_tray(app: &Rc<App>, on: bool) {
    if !on {
        app.tray.borrow_mut().take();
        return;
    }
    if app.tray.borrow().is_some() {
        return;
    }
    let Ok(tray) = ui::Tray::new() else { return };
    tray.on_open(|| {
        with_app(|a| {
            show_main(a);
        })
    });
    tray.on_snip(|m| with_app(move |a| snip(a, &m)));
    tray.on_record(|| with_app(record_toggle));
    tray.on_voice(|| {
        with_app(|a| {
            show_main(a);
            go(a, "voice");
        })
    });
    tray.on_quit(|| {
        with_app(stop_all_now);
        let _ = slint::quit_event_loop();
    });
    let _ = tray.show();
    *app.tray.borrow_mut() = Some(tray);
}
