//! The app: one shared state on the UI thread, reached from callbacks and from worker threads (through
//! `slint::invoke_from_event_loop` and `with_app`).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle as _, ModelRc, SharedString, VecModel};

use crate::files::{self, Kind};
use crate::prefs::{self, Prefs};
use crate::ui;
use crate::{cameractl, capturectl, mediactl, settingsctl, textctl, viewer};

pub struct App {
    pub ui: ui::AppWindow,
    pub prefs: RefCell<Prefs>,
    pub recents: RefCell<Vec<PathBuf>>,
    pub view: RefCell<viewer::ViewState>,
    pub media: RefCell<mediactl::MediaState>,
    pub text: RefCell<textctl::TextState>,
    pub cap: RefCell<capturectl::CaptureState>,
    pub camera: RefCell<cameractl::CameraState>,
    pub tray: RefCell<Option<ui::Tray>>,
    pub hotkeys: RefCell<Option<crate::hotkeys::Hotkeys>>,
    /// A downloaded update waiting to be applied.
    pub update: RefCell<Option<Box<velopack::UpdateInfo>>>,
    /// The setup window of the installer's first run, while it is open.
    pub setup: RefCell<Option<ui::SetupWindow>>,
    /// True when started with --background (no window until asked for).
    pub background_start: Cell<bool>,
    pub toast_timer: slint::Timer,
    pub tick: slint::Timer,
    pub slow_tick: slint::Timer,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// Runs `f` with the app, if it exists (UI thread only).
pub fn with_app(f: impl FnOnce(&Rc<App>)) {
    let app = APP.with(|a| a.borrow().clone());
    if let Some(app) = app {
        f(&app);
    }
}

/// Stops playback and recordings and releases the app while the thread is still alive (sound devices must not be
/// closed during process exit).
pub fn shutdown() {
    let app = APP.with(|a| a.borrow_mut().take());
    if let Some(app) = app {
        app.media.borrow_mut().take_player();
        cameractl::stop_all_now(&app);
        capturectl::stop_all_now(&app);
        app.tray.borrow_mut().take();
        app.hotkeys.borrow_mut().take();
        // Quitting with a downloaded update: it is applied on the way out, so the next start is new.
        if let Some(info) = app.update.borrow_mut().take() {
            crate::update::apply_on_exit(&info);
        }
    }
}

/// Runs `f` with the app on the UI thread, from any thread.
pub fn post(f: impl FnOnce(&Rc<App>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || with_app(f));
}

pub fn build(background: bool) -> Result<Rc<App>, slint::PlatformError> {
    let ui = ui::AppWindow::new()?;
    let app = Rc::new(App {
        ui,
        prefs: RefCell::new(Prefs::load()),
        recents: RefCell::new(prefs::load_recents()),
        view: RefCell::new(viewer::ViewState::default()),
        media: RefCell::new(mediactl::MediaState::default()),
        text: RefCell::new(textctl::TextState::default()),
        cap: RefCell::new(capturectl::CaptureState::default()),
        camera: RefCell::new(cameractl::CameraState::default()),
        tray: RefCell::new(None),
        hotkeys: RefCell::new(None),
        update: RefCell::new(None),
        setup: RefCell::new(None),
        background_start: Cell::new(background),
        toast_timer: slint::Timer::default(),
        tick: slint::Timer::default(),
        slow_tick: slint::Timer::default(),
    });
    APP.with(|a| *a.borrow_mut() = Some(Rc::clone(&app)));
    wire_shell(&app);
    viewer::wire(&app);
    mediactl::wire(&app);
    capturectl::wire(&app);
    cameractl::wire(&app);
    settingsctl::wire(&app);
    textctl::wire(&app);
    refresh_recents(&app);
    settingsctl::apply_theme(&app);
    Ok(app)
}

fn wire_shell(app: &Rc<App>) {
    let shell = app.ui.global::<ui::Shell>();
    shell.on_open_file(|| with_app(open_dialog));
    shell.on_rename(|name| with_app(move |a| rename_current(a, &name)));
    shell.on_open_recent(|i| {
        with_app(|a| {
            if let Some(p) = recent_at(a, i as usize) {
                open_path(a, &p);
            }
        })
    });
    shell.on_clear_recents(|| {
        with_app(|a| {
            a.recents.borrow_mut().clear();
            prefs::save_recents(&[]);
            refresh_recents(a);
        })
    });
    shell.on_go(|page| with_app(|a| go(a, &page)));
    shell.on_toggle_full_screen(|| with_app(toggle_full_screen));
    shell.on_apply_update(|| {
        with_app(|a| {
            let info = a.update.borrow().clone();
            if let Some(info) = info {
                capturectl::stop_all_now(a);
                if let Err(e) = crate::update::apply_now(&info) {
                    toast(a, format!("Could not update: {e}"));
                }
            }
        })
    });
    shell.on_key(|text, ctrl, shift, alt| {
        let mut handled = false;
        with_app(|a| handled = on_key(a, &text, ctrl, shift, alt));
        handled
    });

    shell.on_close_tab(|| with_app(close_tab));
    // The cast button opens the cast popup, which searches for devices (src/cast.rs).
    shell.on_cast(|| with_app(crate::cast::refresh));
    let cast = app.ui.global::<ui::Cast>();
    cast.on_refresh(|| with_app(crate::cast::refresh));
    cast.on_pick(|i| with_app(|a| crate::cast::pick(a, i as usize)));
    cast.on_pause(crate::cast::toggle_play);
    cast.on_stop(crate::cast::stop);
    cast.on_system_cast(crate::platform::open_cast_panel);

    // Closing the window clears every tab (after asking about unsaved changes): it means "I'm done"; minimise keeps
    // things open. Then stay in the tray when "keep running" is on, otherwise quit.
    app.ui.window().on_close_requested(|| {
        let mut response = slint::CloseRequestResponse::KeepWindowShown;
        with_app(|a| {
            if !viewer::confirm_discard(a) {
                return;
            }
            // An unsaved canvas asks in the window first; the close then goes on from the answer.
            if viewer::canvas_unsaved(a) {
                viewer::settle_canvas(a, |a| {
                    // Unsaved text asks in the window; the close then goes on from the answer (finish_close).
                    if textctl::settle(a, textctl::Pending::Quit) {
                        finish_close(a);
                    }
                });
                return;
            }
            // Unsaved text asks in the window; the close then goes on from the answer (finish_close).
            if !textctl::settle(a, textctl::Pending::Quit) {
                return;
            }
            finish_close(a);
            response = slint::CloseRequestResponse::HideWindow;
        });
        response
    });

    // Files dropped on the window open; hovering highlights the home page drop zone.
    use slint::winit_030::{EventResult, WinitWindowAccessor as _, winit};
    app.ui.window().on_winit_window_event(|_w, event| {
        match event {
            winit::event::WindowEvent::HoveredFile(_) => with_app(|a| a.ui.global::<ui::Shell>().set_drop_hover(true)),
            winit::event::WindowEvent::HoveredFileCancelled => with_app(|a| a.ui.global::<ui::Shell>().set_drop_hover(false)),
            winit::event::WindowEvent::DroppedFile(path) => {
                let path = path.clone();
                with_app(move |a| {
                    a.ui.global::<ui::Shell>().set_drop_hover(false);
                    // An image dropped while marking up (or on a canvas) comes in as a picture mark, or the canvas.
                    let markup = a.ui.global::<ui::Shell>().get_page() == "image" && viewer::in_markup(a);
                    if markup && files::kind_of(&path) == Some(Kind::Image) {
                        viewer::drop_picture(a, &path);
                    } else {
                        open_path(a, &path);
                    }
                });
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

pub fn go(app: &Rc<App>, page: &str) {
    let shell = app.ui.global::<ui::Shell>();
    let was = shell.get_page();
    if matches!(was.as_str(), "audio" | "video") && was != page {
        mediactl::leaving(app);
    }
    if was == "camera" && page != "camera" {
        cameractl::leave(app);
    }
    if page == "camera" {
        cameractl::enter(app);
    }
    if page == "voice" {
        capturectl::refresh_takes(app);
    }
    if page == "settings" {
        settingsctl::refresh(app);
    }
    shell.set_page(page.into());
    update_title(app);
}

pub fn update_title(app: &Rc<App>) {
    let shell = app.ui.global::<ui::Shell>();
    // The file's name alone (what the title shows and the rename box edits), then the title with the app name.
    let (title, name) = match shell.get_page().as_str() {
        "image" if app.view.borrow().canvas => ("Canvas".to_string(), String::new()),
        "image" => {
            let name = app.view.borrow().path.as_deref().map(files::file_name).unwrap_or_default();
            (if name.is_empty() { "Picture".to_string() } else { name.clone() }, name)
        }
        "audio" | "video" => {
            let name = app.media.borrow().path.as_deref().map(files::file_name).unwrap_or_default();
            (name.clone(), name)
        }
        "text" => (textctl::title(app), String::new()),
        "voice" => ("Voice recorder".to_string(), String::new()),
        "camera" => ("Camera".to_string(), String::new()),
        "settings" => ("Settings".to_string(), String::new()),
        _ => (String::new(), String::new()),
    };
    shell.set_title_name(name.into());
    shell.set_title(if title.is_empty() { "Rimelight Media".into() } else { format!("Rimelight Media - {title}").into() });
}

/// Renames the file in the Image or Audio/Video tab (Enter in the title). The extension is kept when the name has none.
pub fn rename_current(app: &Rc<App>, typed: &str) {
    let page = app.ui.global::<ui::Shell>().get_page();
    let old = match page.as_str() {
        "image" => app.view.borrow().path.clone(),
        "audio" | "video" => app.media.borrow().path.clone(),
        _ => None,
    };
    let Some(old) = old else {
        toast(app, "Save the picture first, then rename it.");
        return;
    };
    let typed = typed.trim();
    if typed.is_empty() || typed.chars().any(|c| "\\/:*?\"<>|".contains(c)) {
        toast(app, "A name can't be empty or contain \\ / : * ? \" < > |");
        return;
    }
    let mut name = typed.to_string();
    if Path::new(&name).extension().is_none()
        && let Some(ext) = old.extension()
    {
        name.push('.');
        name.push_str(&ext.to_string_lossy());
    }
    let Some(dir) = old.parent() else { return };
    let new = dir.join(&name);
    if new == old {
        return;
    }
    if new.exists() {
        toast(app, format!("{name} is already in this folder."));
        return;
    }
    // A playing file is let go first, so Windows lets it be renamed; it is opened again under the new name.
    let playing = page != "image";
    if playing {
        app.media.borrow_mut().take_player();
    }
    if let Err(e) = std::fs::rename(&old, &new) {
        toast(app, format!("Could not rename: {e}"));
        if playing {
            mediactl::open(app, &old);
        }
        return;
    }
    if playing {
        mediactl::open(app, &new);
    } else {
        viewer::renamed(app, &new);
    }
    {
        let mut list = app.recents.borrow_mut();
        list.retain(|p| p != &old);
        prefs::push_recent(&mut list, &new);
        prefs::save_recents(&list);
    }
    refresh_recents(app);
    update_title(app);
    toast(app, format!("Renamed to {name}"));
}

pub fn toast(app: &Rc<App>, text: impl Into<SharedString>) {
    app.ui.global::<ui::Shell>().set_toast(text.into());
    app.toast_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(3500), || {
        with_app(|a| a.ui.global::<ui::Shell>().set_toast(SharedString::new()))
    });
}

/// Shows and raises the main window.
pub fn show_main(app: &Rc<App>) {
    let _ = app.ui.show();
    use slint::winit_030::WinitWindowAccessor as _;
    app.ui.window().with_winit_window(|w| {
        w.set_minimized(false);
        w.focus_window();
    });
}

pub fn toggle_full_screen(app: &Rc<App>) {
    let shell = app.ui.global::<ui::Shell>();
    let on = !shell.get_full_screen();
    app.ui.window().set_fullscreen(on);
    shell.set_full_screen(on);
}

pub fn open_dialog(app: &Rc<App>) {
    let mut all: Vec<&str> = files::IMAGE_EXTS.to_vec();
    all.extend_from_slice(files::VIDEO_EXTS);
    all.extend_from_slice(files::AUDIO_EXTS);
    all.extend_from_slice(files::TEXT_EXTS);
    let picked = rfd::FileDialog::new()
        .set_title("Open")
        .add_filter("Pictures, video, music and text", &all)
        .add_filter("Pictures", files::IMAGE_EXTS)
        .add_filter("Video", files::VIDEO_EXTS)
        .add_filter("Music", files::AUDIO_EXTS)
        .add_filter("Text", files::TEXT_EXTS)
        .add_filter("Everything", &["*"])
        .pick_files();
    if let Some(paths) = picked {
        open_many(app, &paths);
    }
}

/// Opens the files picked together. Several pictures show only those in the filmstrip; one file browses its folder.
pub fn open_many(app: &Rc<App>, paths: &[PathBuf]) {
    let Some(first) = paths.first() else { return };
    if paths.len() > 1 && paths.iter().all(|p| files::kind_of(p) == Some(Kind::Image)) {
        viewer::open_set(app, paths);
        remember(app, first);
    } else {
        open_path(app, first);
    }
}

pub fn open_path(app: &Rc<App>, path: &Path) {
    match files::kind_of(path) {
        Some(Kind::Image) => viewer::open(app, path),
        Some(Kind::Text) => textctl::open(app, path),
        Some(Kind::Video | Kind::Audio) => mediactl::open(app, path),
        None => {
            // Unknown extension: let FFmpeg try it as media.
            if path.is_file() {
                mediactl::open(app, path);
            } else {
                toast(app, "That file can't be opened.");
                return;
            }
        }
    }
    remember(app, path);
}

pub fn remember(app: &Rc<App>, path: &Path) {
    {
        let mut list = app.recents.borrow_mut();
        prefs::push_recent(&mut list, path);
        prefs::save_recents(&list);
    }
    refresh_recents(app);
}

pub fn refresh_recents(app: &Rc<App>) {
    let items: Vec<ui::RecentItem> = app
        .recents
        .borrow()
        .iter()
        .filter(|p| p.exists())
        .map(|p| ui::RecentItem {
            name: files::file_name(p).into(),
            detail: files::folder_name(p).into(),
            kind: files::kind_of(p).map(files::kind_name).unwrap_or("video").into(),
        })
        .collect();
    app.ui.global::<ui::Shell>().set_recents(ModelRc::new(VecModel::from(items)));
}

/// Recent items shown are the existing ones; map a shown index back to the stored list.
pub fn recent_at(app: &Rc<App>, i: usize) -> Option<PathBuf> {
    app.recents.borrow().iter().filter(|p| p.exists()).nth(i).cloned()
}

fn on_key(app: &Rc<App>, text: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
    use slint::platform::Key;
    let k = |key: Key| text == SharedString::from(key).as_str();
    let page = app.ui.global::<ui::Shell>().get_page();
    if ctrl && (text == "o" || text == "O") {
        open_dialog(app);
        return true;
    }
    // Ctrl+N: the "New canvas" question. Ctrl+V on the home page: the clipboard picture opens as a canvas (the other
    // pages keep their own paste, in the text editor and the picture page).
    if ctrl && (text == "n" || text == "N") {
        app.ui.global::<ui::Shell>().set_canvas_popup_open(true);
        return true;
    }
    if ctrl && (text == "v" || text == "V") && page == "home" {
        viewer::paste_new(app);
        return true;
    }
    if k(Key::F11) {
        toggle_full_screen(app);
        return true;
    }
    if k(Key::Escape) {
        if app.ui.global::<ui::Shell>().get_full_screen() {
            toggle_full_screen(app);
            return true;
        }
        if page == "image" && app.ui.global::<ui::Viewer>().get_comparing() {
            viewer::close_compare(app);
            return true;
        }
        if page == "image" && !app.ui.global::<ui::Viewer>().get_mode().is_empty() {
            viewer::set_mode(app, "");
            return true;
        }
        return false;
    }
    let _ = alt;
    match page.as_str() {
        "image" => viewer::on_key(app, text, ctrl, shift),
        "audio" | "video" => mediactl::on_key(app, text, ctrl, shift),
        _ => false,
    }
}

/// Checks for an update in the background; `manual` reports "up to date" and failures too.
pub fn check_updates(app: &Rc<App>, manual: bool) {
    if app.update.borrow().is_some() {
        return;
    }
    if manual {
        app.ui.global::<ui::Settings>().set_update_state("Checking for updates…".into());
    }
    std::thread::spawn(move || {
        let outcome = crate::update::check_and_download();
        post(move |a| {
            use crate::update::Outcome;
            let text = match outcome {
                Outcome::Off => "Updates are off: this copy isn't installed, or was built without a GitHub repository.".to_string(),
                Outcome::UpToDate => "Up to date.".to_string(),
                Outcome::Failed(e) => e,
                Outcome::Ready(info) => {
                    let v = info.TargetFullRelease.Version.clone();
                    *a.update.borrow_mut() = Some(info);
                    a.ui.global::<ui::Shell>().set_update_ready(v.clone().into());
                    format!("Version {v} is downloaded. Restart to use it (or it is applied when you quit).")
                }
            };
            a.ui.global::<ui::Settings>().set_update_state(text.clone().into());
            if manual {
                toast(a, text);
            }
        });
    });
}
/// (Re)registers the global Print Screen keys and refreshes their status in Settings.
pub fn start_hotkeys(app: &Rc<App>) {
    app.hotkeys.borrow_mut().take();
    let keys = crate::hotkeys::start(|action| with_app(|a| capturectl::hotkey(a, action)));
    *app.hotkeys.borrow_mut() = keys;
    settingsctl::refresh(app);
}
/// Centres a window on the monitor the pointer is on (Windows opens new windows at its own default spot).
pub fn center_window(window: &slint::Window) {
    let size = window.size();
    if let Some((_, mon)) = crate::platform::cursor_monitor() {
        let x = mon.x + (mon.w - size.width as i32).max(0) / 2;
        let y = mon.y + (mon.h - size.height as i32).max(0) / 2;
        window.set_position(slint::PhysicalPosition::new(x, y));
    }
}

/// The main window's handle (owner of the print and share dialogs).
pub fn main_hwnd(app: &Rc<App>) -> isize {
    use slint::winit_030::WinitWindowAccessor as _;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    app.ui
        .window()
        .with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => 0,
        })
        .unwrap_or(0)
}
/// The X in the top bar: back to the empty version of this tab (asking first about unsaved changes).
pub fn close_tab(app: &Rc<App>) {
    match app.ui.global::<ui::Shell>().get_page().as_str() {
        "image" => viewer::close_tab(app),
        "audio" | "video" => mediactl::close(app),
        "text" => textctl::close(app),
        _ => {}
    }
    update_title(app);
}

/// The window is closing, with nothing left unsaved: empties the tabs, then quits, or hides the window when the app keeps
/// running in the tray.
pub fn finish_close(app: &Rc<App>) {
    clear_all(app);
    if !app.prefs.borrow().background {
        capturectl::stop_all_now(app);
        let _ = slint::quit_event_loop();
    }
    let _ = app.ui.window().hide();
}

/// Empties every tab and frees what they held (the window was closed).
fn clear_all(app: &Rc<App>) {
    viewer::close(app);
    mediactl::close(app);
    textctl::clear(app);
    if app.ui.global::<ui::Shell>().get_full_screen() {
        toggle_full_screen(app);
    }
    go(app, "home");
}

/// A request from a later start (single instance): a file, a snip, or just the window.
pub fn handle_request(request: crate::instance::Request) {
    use crate::instance::Request;
    with_app(|a| match request {
        Request::Show => show_main(a),
        Request::Open(p) => {
            show_main(a);
            queue_open(a, PathBuf::from(p));
        }
        Request::Snip(mode) => capturectl::snip(a, &mode),
        Request::Nothing => {}
    });
}
thread_local! {
    static PENDING_OPENS: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Explorer starts the app once per file when several are opened together; the starts arrive here a few ms apart. They
/// are gathered for a moment and opened as one set, so the filmstrip shows just those files.
pub fn queue_open(app: &Rc<App>, path: PathBuf) {
    let first = PENDING_OPENS.with(|p| {
        let mut p = p.borrow_mut();
        p.push(path);
        p.len() == 1
    });
    if first {
        let app = app.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(350), move || {
            let mut paths = PENDING_OPENS.with(|p| std::mem::take(&mut *p.borrow_mut()));
            paths.dedup();
            open_many(&app, &paths);
        });
    }
}
