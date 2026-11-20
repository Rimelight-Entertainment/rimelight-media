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
use crate::{capturectl, mediactl, settingsctl, viewer};

pub struct App {
    pub ui: ui::AppWindow,
    pub prefs: RefCell<Prefs>,
    pub recents: RefCell<Vec<PathBuf>>,
    pub view: RefCell<viewer::ViewState>,
    pub media: RefCell<mediactl::MediaState>,
    pub cap: RefCell<capturectl::CaptureState>,
    pub tray: RefCell<Option<ui::Tray>>,
    pub hotkeys: RefCell<Option<crate::hotkeys::Hotkeys>>,
    /// A downloaded update waiting to be applied.
    pub update: RefCell<Option<Box<velopack::UpdateInfo>>>,
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
        cap: RefCell::new(capturectl::CaptureState::default()),
        tray: RefCell::new(None),
        hotkeys: RefCell::new(None),
        update: RefCell::new(None),
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
    settingsctl::wire(&app);
    refresh_recents(&app);
    settingsctl::apply_theme(&app);
    Ok(app)
}

fn wire_shell(app: &Rc<App>) {
    let shell = app.ui.global::<ui::Shell>();
    shell.on_open_file(|| with_app(open_dialog));
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

    // Closing the window: stay in the tray when "keep running" is on, otherwise quit.
    app.ui.window().on_close_requested(|| {
        let mut response = slint::CloseRequestResponse::HideWindow;
        with_app(|a| {
            mediactl::pause(a);
            if !a.prefs.borrow().background {
                capturectl::stop_all_now(a);
                let _ = slint::quit_event_loop();
            } else {
                response = slint::CloseRequestResponse::HideWindow;
            }
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
                    open_path(a, &path);
                });
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

pub fn go(app: &Rc<App>, page: &str) {
    let shell = app.ui.global::<ui::Shell>();
    if shell.get_page() == "media" && page != "media" {
        mediactl::leaving(app);
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
    let title = match shell.get_page().as_str() {
        "image" => app.view.borrow().path.as_deref().map(files::file_name).unwrap_or_else(|| "Picture".into()),
        "media" => app.media.borrow().path.as_deref().map(files::file_name).unwrap_or_default(),
        "voice" => "Voice recorder".into(),
        "settings" => "Settings".into(),
        _ => String::new(),
    };
    shell.set_title(if title.is_empty() { "Rimelight Media".into() } else { format!("{title} - Rimelight Media").into() });
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
    let picked = rfd::FileDialog::new()
        .set_title("Open")
        .add_filter("Pictures, video and music", &all)
        .add_filter("Pictures", files::IMAGE_EXTS)
        .add_filter("Video", files::VIDEO_EXTS)
        .add_filter("Music", files::AUDIO_EXTS)
        .add_filter("Everything", &["*"])
        .pick_file();
    if let Some(path) = picked {
        open_path(app, &path);
    }
}

pub fn open_path(app: &Rc<App>, path: &Path) {
    match files::kind_of(path) {
        Some(Kind::Image) => viewer::open(app, path),
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
    if k(Key::F11) {
        toggle_full_screen(app);
        return true;
    }
    if k(Key::Escape) {
        if app.ui.global::<ui::Shell>().get_full_screen() {
            toggle_full_screen(app);
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
        "media" => mediactl::on_key(app, text, ctrl, shift),
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