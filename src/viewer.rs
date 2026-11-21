//! The picture page: open (in the background), browse the folder, zoom, crop, rotate, flip, resize, markup and save.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{ComponentHandle as _, Model as _, ModelRc, SharedString, VecModel};

use crate::app::{App, go, post, remember, toast, update_title, with_app};
use crate::files;
use crate::picture::{self, Mark, Picture};
use crate::ui;

#[derive(Default)]
pub struct ViewState {
    pub path: Option<PathBuf>,
    original: Option<Picture>,
    pic: Option<Picture>,
    folder: Vec<PathBuf>,
    index: usize,
    marks: Vec<Mark>,
    redo: Vec<Mark>,
    drawing: Option<Mark>,
    text_at: Option<(f32, f32)>,
    generation: u64,
    file_size: u64,
    /// Frames of an animated GIF or WebP, their pictures for the screen (made on first show) and the one showing.
    anim: Option<Rc<picture::Animation>>,
    anim_images: Vec<Option<slint::Image>>,
    anim_frame: usize,
    anim_playing: bool,
    anim_timer: slint::Timer,
    /// Earlier versions of the picture (Undo) and undone ones (Redo), newest last.
    history: Vec<Picture>,
    future: Vec<Picture>,
    /// Filmstrip: its thumbnails (for the folder in thumbs_dir) and the pictures picked to compare.
    thumbs: Option<Rc<VecModel<ui::Thumb>>>,
    thumbs_dir: Option<PathBuf>,
    thumbs_gen: u64,
    picked: Vec<usize>,
}

/// Picture edits kept for Undo.
const MAX_HISTORY: usize = 20;

fn viewer(app: &Rc<App>) -> ui::Viewer<'_> {
    app.ui.global::<ui::Viewer>()
}

pub fn wire(app: &Rc<App>) {
    let v = viewer(app);
    v.on_prev(|| with_app(|a| step(a, -1)));
    v.on_next(|| with_app(|a| step(a, 1)));
    v.on_rotate(|| with_app(|a| edit(a, picture::rotate_cw)));
    v.on_flip(|lr| with_app(move |a| edit(a, move |p| picture::flip(p, lr))));
    v.on_set_mode(|m| with_app(|a| set_mode(a, &m)));
    v.on_apply_crop(|| {
        with_app(|a| {
            let v = viewer(a);
            let (x, y, w, h) = (v.get_crop_x(), v.get_crop_y(), v.get_crop_w(), v.get_crop_h());
            edit(a, move |p| picture::crop(p, x, y, w, h));
            set_mode(a, "");
        })
    });
    v.on_crop_preset(|aspect| with_app(move |a| crop_preset(a, aspect)));
    v.on_resize_edited(|width_changed| with_app(move |a| resize_edited(a, width_changed)));
    v.on_apply_resize(|| {
        with_app(|a| {
            let v = viewer(a);
            let (Ok(w), Ok(h)) = (v.get_resize_w().trim().parse::<u32>(), v.get_resize_h().trim().parse::<u32>()) else {
                toast(a, "Type a width and a height in pixels.");
                return;
            };
            if w == 0 || h == 0 || w > 30_000 || h > 30_000 {
                toast(a, "Width and height must be between 1 and 30000 pixels.");
                return;
            }
            edit(a, move |p| picture::resize(p, w, h));
            set_mode(a, "");
        })
    });
    v.on_mark_begin(|x, y| with_app(move |a| mark_begin(a, x, y)));
    v.on_mark_move(|x, y| with_app(move |a| mark_move(a, x, y)));
    v.on_mark_end(|| with_app(mark_end));
    v.on_mark_text(|t| with_app(move |a| mark_text(a, &t)));
    v.on_undo(|| with_app(undo));
    v.on_redo(|| with_app(redo));
    v.on_print(|| with_app(print));
    v.on_share(|| with_app(share));
    v.on_toggle_filmstrip(|| with_app(toggle_filmstrip));
    v.on_thumb_clicked(|i, add| with_app(move |a| thumb_clicked(a, i as usize, add)));
    v.on_close_compare(|| with_app(close_compare));
    v.on_clear_marks(|| {
        with_app(|a| {
            let mut s = a.view.borrow_mut();
            let mut cleared = std::mem::take(&mut s.marks);
            cleared.reverse();
            s.redo = cleared;
            drop(s);
            refresh_marks(a);
        })
    });
    v.on_apply_marks(|| {
        with_app(|a| {
            let marks = std::mem::take(&mut a.view.borrow_mut().marks);
            if !marks.is_empty() {
                edit(a, move |p| picture::burn_marks(p, &marks).unwrap_or_else(|_| p.clone()));
            }
            set_mode(a, "");
        })
    });
    v.on_save(|| with_app(save));
    v.on_save_as(|| with_app(save_as));
    v.on_copy(|| with_app(copy));
    v.on_revert(|| {
        with_app(|a| {
            let orig = a.view.borrow().original.clone();
            if let Some(p) = orig {
                a.view.borrow_mut().pic = Some(p);
                show_picture(a, false);
                viewer(a).set_dirty(false);
            }
        })
    });
    v.on_toggle_info(|| with_app(|a| viewer(a).set_show_info(!viewer(a).get_show_info())));
    v.on_anim_toggle(|| with_app(anim_toggle));
    v.on_anim_step(|d| with_app(move |a| anim_step(a, d)));
    v.on_save_frame(|| with_app(save_frame));
    v.on_delete_current(|| with_app(delete_current));
    v.on_open_folder(|| {
        with_app(|a| {
            if let Some(p) = a.view.borrow().path.clone() {
                files::reveal(&p);
            }
        })
    });
}

/// Opens a picture file: the folder list is built at once, the pixels arrive from a worker thread.
pub fn open(app: &Rc<App>, path: &Path) {
    let folder = files::sibling_pictures(path);
    let index = folder.iter().position(|p| p == path).unwrap_or(0);
    let generation = {
        let mut s = app.view.borrow_mut();
        s.generation += 1;
        s.path = Some(path.to_path_buf());
        s.folder = folder;
        s.index = index;
        s.pic = None;
        s.original = None;
        s.marks.clear();
        s.redo.clear();
        s.file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        s.history.clear();
        s.future.clear();
        stop_anim(&mut s);
        s.generation
    };
    viewer(app).set_animated(false);
    let v = viewer(app);
    v.set_loading(true);
    v.set_error(SharedString::new());
    v.set_mode(SharedString::new());
    v.set_dirty(false);
    v.set_fit(true);
    v.set_pan_x(0.0);
    v.set_pan_y(0.0);
    v.set_info_name(files::file_name(path).into());
    update_position(app);
    refresh_marks(app);
    go(app, "image");
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let animation = picture::load_animation(&p);
        let result = match &animation {
            Some(anim) => Ok(anim.frames[0].0.clone()),
            None => picture::load(&p),
        };
        post(move |a| {
            if a.view.borrow().generation != generation {
                return;
            }
            viewer(a).set_loading(false);
            match result {
                Ok(pic) => {
                    {
                        let mut s = a.view.borrow_mut();
                        s.original = Some(pic.clone());
                        s.pic = Some(pic);
                    }
                    show_picture(a, true);
                    if let Some(anim) = animation {
                        start_anim(a, anim);
                    }
                }
                Err(e) => {
                    viewer(a).set_picture(slint::Image::default());
                    viewer(a).set_error(format!("This picture can't be shown.\n{e}").into());
                }
            }
        });
    });
}

/// Shows a picture that isn't a file yet (a fresh snip). `path` is where it was saved, if it was.
pub fn open_picture(app: &Rc<App>, pic: Picture, path: Option<PathBuf>) {
    {
        let mut s = app.view.borrow_mut();
        s.generation += 1;
        s.folder = path.as_ref().map(|p| files::sibling_pictures(p)).unwrap_or_default();
        s.index = path.as_ref().and_then(|p| s.folder.iter().position(|f| f == p)).unwrap_or(0);
        s.file_size = path.as_ref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len()).unwrap_or(0);
        s.path = path.clone();
        s.original = Some(pic.clone());
        s.pic = Some(pic);
        s.marks.clear();
        s.redo.clear();
        s.history.clear();
        s.future.clear();
    }
    let v = viewer(app);
    v.set_loading(false);
    v.set_error(SharedString::new());
    v.set_mode(SharedString::new());
    v.set_dirty(path.is_none());
    v.set_fit(true);
    v.set_info_name(path.as_deref().map(files::file_name).unwrap_or_else(|| "Snip".into()).into());
    update_position(app);
    refresh_marks(app);
    show_picture(app, true);
    go(app, "image");
    if let Some(p) = &path {
        remember(app, p);
    }
}

fn show_picture(app: &Rc<App>, fresh: bool) {
    let s = app.view.borrow();
    let Some(pic) = s.pic.as_ref() else { return };
    let v = viewer(app);
    v.set_picture(picture::to_slint(pic));
    v.set_pic_w(pic.w as i32);
    v.set_pic_h(pic.h as i32);
    if fresh {
        v.set_fit(true);
    }
    v.set_dims_text(format!("{} x {}", pic.w, pic.h).into());
    v.set_file_type(s.path.as_deref().map(|p| files::ext_of(p).to_uppercase()).unwrap_or_else(|| "Not saved".into()).into());
    v.set_size_text(if s.path.is_some() && s.file_size > 0 { files::human_size(s.file_size).into() } else { SharedString::new() });
    v.set_can_share(s.path.as_ref().is_some_and(|p| p.exists()));
    let mut detail = format!("{} × {} pixels", pic.w, pic.h);
    if let Some(p) = &s.path {
        if s.file_size > 0 {
            detail.push_str(&format!("\n{}", files::human_size(s.file_size)));
        }
        detail.push_str(&format!("\n{}", files::ext_of(p).to_uppercase()));
        if let Ok(modified) = std::fs::metadata(p).and_then(|m| m.modified())
            && let Ok(age) = modified.elapsed()
        {
            let days = age.as_secs() / 86_400;
            detail.push_str(&match days {
                0 => "\nChanged today".to_string(),
                1 => "\nChanged yesterday".to_string(),
                d => format!("\nChanged {d} days ago"),
            });
        }
        detail.push_str(&format!("\n\n{}", files::folder_name(p)));
    } else {
        detail.push_str("\nNot saved yet");
    }
    v.set_info_detail(detail.into());
    drop(s);
    update_title(app);
}

fn update_position(app: &Rc<App>) {
    sync_thumbs(app);
    let s = app.view.borrow();
    let v = viewer(app);
    let n = s.folder.len();
    v.set_position(if n > 1 { format!("{} / {n}", s.index + 1).into() } else { SharedString::new() });
    v.set_has_prev(n > 1 && s.index > 0);
    v.set_has_next(n > 1 && s.index + 1 < n);
}

pub fn step(app: &Rc<App>, delta: i32) {
    let next = {
        let s = app.view.borrow();
        let i = s.index as i64 + i64::from(delta);
        (i >= 0 && (i as usize) < s.folder.len()).then(|| s.folder[i as usize].clone())
    };
    if viewer(app).get_dirty() {
        toast(app, "Changes not saved: save them, or undo them, first.");
        return;
    }
    if let Some(p) = next {
        open(app, &p);
    }
}

fn edit(app: &Rc<App>, f: impl FnOnce(&Picture) -> Picture) {
    if app.view.borrow().anim.is_some() {
        freeze_anim(app);
        toast(app, "Editing keeps only this frame, as a still picture.");
    }
    let changed = {
        let mut s = app.view.borrow_mut();
        match s.pic.take() {
            Some(p) => {
                s.pic = Some(f(&p));
                s.history.push(p);
                if s.history.len() > MAX_HISTORY {
                    s.history.remove(0);
                }
                s.future.clear();
                true
            }
            None => false,
        }
    };
    if changed {
        viewer(app).set_dirty(true);
        show_picture(app, false);
        refresh_undo(app);
    }
}

pub fn set_mode(app: &Rc<App>, mode: &str) {
    let v = viewer(app);
    {
        let mut s = app.view.borrow_mut();
        s.marks.clear();
        s.redo.clear();
        s.drawing = None;
        s.text_at = None;
    }
    v.set_text_x(-1.0);
    v.set_text_y(-1.0);
    match mode {
        "crop" => {
            v.set_crop_aspect(0.0);
            v.set_crop_x(0.0);
            v.set_crop_y(0.0);
            v.set_crop_w(1.0);
            v.set_crop_h(1.0);
        }
        "resize" => {
            v.set_resize_w(v.get_pic_w().to_string().into());
            v.set_resize_h(v.get_pic_h().to_string().into());
        }
        _ => {}
    }
    v.set_mode(mode.into());
    refresh_marks(app);
}

fn crop_preset(app: &Rc<App>, aspect: f32) {
    let v = viewer(app);
    let (w, h) = (v.get_pic_w() as f32, v.get_pic_h() as f32);
    let aspect = if aspect < 0.0 { w / h } else { aspect };
    v.set_crop_aspect(aspect);
    if aspect <= 0.0 {
        return;
    }
    let pic_aspect = w / h;
    let (fw, fh) = if aspect > pic_aspect { (1.0, pic_aspect / aspect) } else { (aspect / pic_aspect, 1.0) };
    v.set_crop_w(fw);
    v.set_crop_h(fh);
    v.set_crop_x((1.0 - fw) / 2.0);
    v.set_crop_y((1.0 - fh) / 2.0);
}

fn resize_edited(app: &Rc<App>, width_changed: bool) {
    let v = viewer(app);
    if !v.get_resize_keep() {
        return;
    }
    let (pw, ph) = (v.get_pic_w() as f64, v.get_pic_h() as f64);
    if width_changed {
        if let Ok(w) = v.get_resize_w().trim().parse::<f64>() {
            v.set_resize_h(((w * ph / pw).round().max(1.0) as u32).to_string().into());
        }
    } else if let Ok(h) = v.get_resize_h().trim().parse::<f64>() {
        v.set_resize_w(((h * pw / ph).round().max(1.0) as u32).to_string().into());
    }
}

fn ink(app: &Rc<App>) -> [u8; 4] {
    let c = viewer(app).get_ink();
    [c.red(), c.green(), c.blue(), c.alpha()]
}

fn mark_begin(app: &Rc<App>, x: f32, y: f32) {
    let v = viewer(app);
    let tool = v.get_tool();
    let width = v.get_ink_width();
    // Marks scale with the picture so they read the same on a small and a large picture.
    let scale = (v.get_pic_w().max(v.get_pic_h()) as f32 / 1600.0).max(1.0);
    match tool {
        5 => {
            let mut s = app.view.borrow_mut();
            let reach = 16.0 * scale;
            if let Some((i, _)) = s
                .marks
                .iter()
                .enumerate()
                .map(|(i, m)| (i, m.distance(x, y)))
                .filter(|(_, d)| *d <= reach)
                .min_by(|a, b| a.1.total_cmp(&b.1))
            {
                let m = s.marks.remove(i);
                s.redo.push(m);
            }
            drop(s);
            refresh_marks(app);
        }
        4 => {
            app.view.borrow_mut().text_at = Some((x, y));
            v.set_text_x(x);
            v.set_text_y(y);
        }
        _ => {
            let mut rgba = ink(app);
            if tool == 1 {
                rgba[3] = 110;
            }
            app.view.borrow_mut().drawing = Some(Mark { kind: tool, rgba, width: width * scale, points: vec![(x, y), (x, y)], text: String::new() });
            refresh_marks(app);
        }
    }
}

fn mark_move(app: &Rc<App>, x: f32, y: f32) {
    let mut s = app.view.borrow_mut();
    let Some(m) = s.drawing.as_mut() else { return };
    if m.kind == 0 || m.kind == 1 {
        let last = *m.points.last().unwrap_or(&(x, y));
        if (last.0 - x).hypot(last.1 - y) < 1.5 {
            return;
        }
        m.points.push((x, y));
    } else if let Some(last) = m.points.last_mut() {
        *last = (x, y);
    }
    drop(s);
    refresh_marks(app);
}

fn mark_end(app: &Rc<App>) {
    let mut s = app.view.borrow_mut();
    if let Some(m) = s.drawing.take() {
        let (a, b) = (m.points[0], *m.points.last().unwrap_or(&m.points[0]));
        let tiny = (a.0 - b.0).hypot(a.1 - b.1) < 3.0;
        if !(tiny && (m.kind == 2 || m.kind == 3)) {
            s.marks.push(m);
            s.redo.clear();
        }
    }
    drop(s);
    refresh_marks(app);
}

fn mark_text(app: &Rc<App>, text: &str) {
    let v = viewer(app);
    let scale = (v.get_pic_w().max(v.get_pic_h()) as f32 / 1600.0).max(1.0);
    let width = v.get_ink_width() * scale;
    let rgba = ink(app);
    {
        let mut s = app.view.borrow_mut();
        if let Some(pos) = s.text_at.take()
            && !text.trim().is_empty()
        {
            s.marks.push(Mark { kind: 4, rgba, width, points: vec![pos], text: text.to_string() });
            s.redo.clear();
        }
    }
    v.set_text_x(-1.0);
    v.set_text_y(-1.0);
    refresh_marks(app);
}

/// Undo: the last mark while drawing, otherwise the last edit of the picture.
fn undo(app: &Rc<App>) {
    if viewer(app).get_mode() == "markup" {
        let mut s = app.view.borrow_mut();
        if let Some(m) = s.marks.pop() {
            s.redo.push(m);
        }
        drop(s);
        refresh_marks(app);
        return;
    }
    let changed = {
        let mut s = app.view.borrow_mut();
        match (s.history.pop(), s.pic.take()) {
            (Some(prev), Some(cur)) => {
                s.future.push(cur);
                s.pic = Some(prev);
                true
            }
            (prev, cur) => {
                s.pic = cur;
                if let Some(p) = prev {
                    s.history.push(p);
                }
                false
            }
        }
    };
    if changed {
        let dirty = !app.view.borrow().history.is_empty();
        viewer(app).set_dirty(dirty);
        show_picture(app, false);
    }
    refresh_undo(app);
}

fn redo(app: &Rc<App>) {
    if viewer(app).get_mode() == "markup" {
        let mut s = app.view.borrow_mut();
        if let Some(m) = s.redo.pop() {
            s.marks.push(m);
        }
        drop(s);
        refresh_marks(app);
        return;
    }
    let changed = {
        let mut s = app.view.borrow_mut();
        match (s.future.pop(), s.pic.take()) {
            (Some(next), Some(cur)) => {
                s.history.push(cur);
                s.pic = Some(next);
                true
            }
            (next, cur) => {
                s.pic = cur;
                if let Some(n) = next {
                    s.future.push(n);
                }
                false
            }
        }
    };
    if changed {
        viewer(app).set_dirty(true);
        show_picture(app, false);
    }
    refresh_undo(app);
}

/// Undo and Redo follow the marks while drawing, the picture's edits otherwise.
fn refresh_undo(app: &Rc<App>) {
    let s = app.view.borrow();
    let v = viewer(app);
    if v.get_mode() == "markup" {
        v.set_can_undo(!s.marks.is_empty());
        v.set_can_redo(!s.redo.is_empty());
    } else {
        v.set_can_undo(!s.history.is_empty());
        v.set_can_redo(!s.future.is_empty());
    }
}

fn to_ui(m: &Mark) -> ui::Mark {
    let [r, g, b, a] = m.rgba;
    let (x, y) = m.points.first().copied().unwrap_or_default();
    ui::Mark {
        kind: m.kind,
        colour: slint::Color::from_argb_u8(a, r, g, b),
        width: if m.kind == 4 { m.font_size() } else { m.stroke_width() },
        commands: m.commands().into(),
        x,
        y,
        text: m.text.clone().into(),
    }
}

fn refresh_marks(app: &Rc<App>) {
    let s = app.view.borrow();
    let items: Vec<ui::Mark> = s.marks.iter().chain(s.drawing.iter()).map(to_ui).collect();
    viewer(app).set_marks(ModelRc::new(VecModel::from(items)));
    drop(s);
    refresh_undo(app);
}

fn current(app: &Rc<App>) -> Option<Picture> {
    let s = app.view.borrow();
    let pic = s.pic.clone()?;
    // Unapplied marks are part of what the person sees: include them.
    if !s.marks.is_empty() {
        return picture::burn_marks(&pic, &s.marks).ok();
    }
    Some(pic)
}

fn save(app: &Rc<App>) {
    let path = app.view.borrow().path.clone();
    match path {
        Some(p) if picture::can_save(&p) => write_to(app, &p),
        _ => save_as(app),
    }
}

fn save_as(app: &Rc<App>) {
    let (dir, stem) = {
        let s = app.view.borrow();
        match &s.path {
            Some(p) => (p.parent().map(Path::to_path_buf), p.file_stem().and_then(|s| s.to_str()).unwrap_or("Picture").to_string()),
            None => (Some(app.prefs.borrow().screenshot_dir.clone()), files::stamp("Screenshot")),
        }
    };
    let ext = app.view.borrow().path.as_deref().map(files::ext_of).filter(|e| matches!(e.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp")).unwrap_or_else(|| "png".into());
    let mut dialog = rfd::FileDialog::new()
        .set_title("Save as")
        .set_file_name(format!("{stem}.{ext}"))
        .add_filter("PNG", &["png"])
        .add_filter("JPEG", &["jpg", "jpeg"])
        .add_filter("WebP (lossless)", &["webp"])
        .add_filter("BMP", &["bmp"]);
    if let Some(d) = dir {
        dialog = dialog.set_directory(d);
    }
    if let Some(mut path) = dialog.save_file() {
        if !picture::can_save(&path) {
            path.set_extension("png");
        }
        write_to(app, &path);
    }
}

fn write_to(app: &Rc<App>, path: &Path) {
    let Some(pic) = current(app) else { return };
    match picture::save(&pic, path) {
        Ok(()) => {
            {
                let mut s = app.view.borrow_mut();
                s.marks.clear();
                s.original = Some(pic.clone());
                s.pic = Some(pic);
                s.path = Some(path.to_path_buf());
                s.file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                s.folder = files::sibling_pictures(path);
                s.index = s.folder.iter().position(|p| p == path).unwrap_or(0);
            }
            let v = viewer(app);
            v.set_dirty(false);
            v.set_info_name(files::file_name(path).into());
            set_mode(app, "");
            show_picture(app, false);
            update_position(app);
            remember(app, path);
            toast(app, format!("Saved {}", files::file_name(path)));
        }
        Err(e) => toast(app, format!("Could not save: {e}")),
    }
}

fn copy(app: &Rc<App>) {
    let Some(pic) = current(app) else { return };
    match picture::copy_to_clipboard(&pic) {
        Ok(()) => toast(app, "Copied to the clipboard"),
        Err(e) => toast(app, format!("Could not copy: {e}")),
    }
}

/// Zooms by a factor around the middle of the stage.
fn zoom(app: &Rc<App>, factor: f32) {
    let v = viewer(app);
    let current = if v.get_fit() {
        let size = app.ui.window().size().to_logical(app.ui.window().scale_factor());
        let info = if v.get_show_info() { 280.0 } else { 0.0 };
        let sw = (size.width - info - 48.0).max(1.0);
        let sh = (size.height - 88.0 - 48.0).max(1.0);
        (sw / v.get_pic_w() as f32).min(sh / v.get_pic_h() as f32).min(1.0)
    } else {
        v.get_scale()
    };
    let new = (current * factor).clamp(0.02, 40.0);
    let ratio = new / current;
    v.set_pan_x(if v.get_fit() { 0.0 } else { v.get_pan_x() * ratio });
    v.set_pan_y(if v.get_fit() { 0.0 } else { v.get_pan_y() * ratio });
    v.set_scale(new);
    v.set_fit(false);
}

pub fn on_key(app: &Rc<App>, text: &str, ctrl: bool, shift: bool) -> bool {
    use slint::platform::Key;
    let k = |key: Key| text == SharedString::from(key).as_str();
    let v = viewer(app);
    let mode = v.get_mode();
    if v.get_animated() && mode.is_empty() && !ctrl {
        if text == " " {
            anim_toggle(app);
            return true;
        }
        if text == "," {
            anim_step(app, -1);
            return true;
        }
        if text == "." {
            anim_step(app, 1);
            return true;
        }
    }
    if ctrl {
        match text.to_ascii_lowercase().as_str() {
            "c" => copy(app),
            "s" if shift => save_as(app),
            "s" => save(app),
            "z" => undo(app),
            "y" => redo(app),
            "p" => print(app),
            _ => return false,
        }
        return true;
    }
    if !mode.is_empty() {
        if k(Key::Return) {
            match mode.as_str() {
                "crop" => v.invoke_apply_crop(),
                "markup" => v.invoke_apply_marks(),
                "resize" => v.invoke_apply_resize(),
                _ => {}
            }
            return true;
        }
        return false;
    }
    if k(Key::Delete) {
        delete_current(app);
    } else if k(Key::LeftArrow) || k(Key::PageUp) {
        step(app, -1);
    } else if k(Key::RightArrow) || k(Key::PageDown) || text == " " {
        step(app, 1);
    } else if k(Key::Home) {
        let first = app.view.borrow().folder.first().cloned();
        if let Some(p) = first {
            open(app, &p);
        }
    } else if k(Key::End) {
        let last = app.view.borrow().folder.last().cloned();
        if let Some(p) = last {
            open(app, &p);
        }
    } else if text == "+" || text == "=" {
        zoom(app, 1.25);
    } else if text == "-" {
        zoom(app, 1.0 / 1.25);
    } else if text == "0" {
        v.set_fit(true);
    } else if text == "1" {
        v.set_scale(1.0);
        v.set_pan_x(0.0);
        v.set_pan_y(0.0);
        v.set_fit(false);
    } else if text == "r" || text == "R" {
        edit(app, picture::rotate_cw);
    } else if text == "c" {
        set_mode(app, "crop");
    } else if text == "m" {
        set_mode(app, "markup");
    } else if text == "i" {
        v.set_show_info(!v.get_show_info());
    } else {
        return false;
    }
    true
}

fn stop_anim(s: &mut ViewState) {
    s.anim_timer.stop();
    s.anim = None;
    s.anim_images.clear();
    s.anim_frame = 0;
    s.anim_playing = false;
}

fn start_anim(app: &Rc<App>, anim: picture::Animation) {
    let n = anim.frames.len();
    let cut = anim.cut;
    {
        let mut s = app.view.borrow_mut();
        s.anim_images = vec![None; n];
        s.anim = Some(Rc::new(anim));
        s.anim_frame = 0;
        s.anim_playing = true;
    }
    let v = viewer(app);
    v.set_animated(true);
    if cut {
        toast(app, "This animation is very large: only its first part is shown.");
    }
    show_frame(app, 0);
    schedule_next(app);
}

/// Shows frame `i` (its screen picture is made the first time).
fn show_frame(app: &Rc<App>, i: usize) {
    let (image, text, playing) = {
        let mut s = app.view.borrow_mut();
        let Some(anim) = s.anim.clone() else { return };
        let i = i % anim.frames.len();
        s.anim_frame = i;
        let image = match s.anim_images[i].clone() {
            Some(img) => img,
            None => {
                let img = picture::to_slint(&anim.frames[i].0);
                s.anim_images[i] = Some(img.clone());
                img
            }
        };
        (image, format!("{} / {}  ·  {} ms", i + 1, anim.frames.len(), anim.frames[i].1), s.anim_playing)
    };
    let v = viewer(app);
    v.set_picture(image);
    v.set_frame_text(text.into());
    v.set_anim_playing(playing);
}

fn schedule_next(app: &Rc<App>) {
    let s = app.view.borrow();
    let Some(anim) = s.anim.as_ref() else { return };
    if !s.anim_playing {
        return;
    }
    let delay = anim.frames[s.anim_frame].1;
    let generation = s.generation;
    s.anim_timer.start(slint::TimerMode::SingleShot, std::time::Duration::from_millis(u64::from(delay)), move || {
        with_app(move |a| {
            if a.view.borrow().generation != generation || !a.view.borrow().anim_playing {
                return;
            }
            let next = a.view.borrow().anim_frame + 1;
            show_frame(a, next);
            schedule_next(a);
        })
    });
}

fn anim_toggle(app: &Rc<App>) {
    {
        let mut s = app.view.borrow_mut();
        if s.anim.is_none() {
            return;
        }
        s.anim_playing = !s.anim_playing;
        if !s.anim_playing {
            s.anim_timer.stop();
        }
    }
    let frame = app.view.borrow().anim_frame;
    show_frame(app, frame);
    schedule_next(app);
}

/// One frame back or forward (pauses the animation).
fn anim_step(app: &Rc<App>, delta: i32) {
    let next = {
        let mut s = app.view.borrow_mut();
        let Some(anim) = s.anim.as_ref() else { return };
        let n = anim.frames.len() as i64;
        s.anim_playing = false;
        s.anim_timer.stop();
        ((s.anim_frame as i64 + i64::from(delta)).rem_euclid(n)) as usize
    };
    show_frame(app, next);
}

/// Keeps the frame showing as a still picture (before an edit).
fn freeze_anim(app: &Rc<App>) {
    let mut s = app.view.borrow_mut();
    if let Some(anim) = s.anim.clone() {
        s.pic = Some(anim.frames[s.anim_frame].0.clone());
    }
    stop_anim(&mut s);
    drop(s);
    viewer(app).set_animated(false);
}

fn save_frame(app: &Rc<App>) {
    let (pic, path, frame) = {
        let s = app.view.borrow();
        let Some(anim) = s.anim.as_ref() else { return };
        (anim.frames[s.anim_frame].0.clone(), s.path.clone(), s.anim_frame)
    };
    let dir = path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| app.prefs.borrow().screenshot_dir.clone());
    let stem = path.as_ref().and_then(|p| p.file_stem()).and_then(|s| s.to_str()).unwrap_or("Animation").to_string();
    let out = files::free_path(&dir, &format!("{stem} frame {}", frame + 1), "png");
    match picture::save(&pic, &out) {
        Ok(()) => toast(app, format!("Frame saved: {}", files::file_name(&out))),
        Err(e) => toast(app, format!("Could not save the frame: {e}")),
    }
}
/// Delete: a file goes to the Recycle Bin and the next picture in the folder opens; a snip that was never saved is
/// discarded.
fn delete_current(app: &Rc<App>) {
    let (path, next) = {
        let s = app.view.borrow();
        let next = s
            .folder
            .get(s.index + 1)
            .or_else(|| s.index.checked_sub(1).and_then(|i| s.folder.get(i)))
            .cloned();
        (s.path.clone(), next)
    };
    match path {
        Some(p) => {
            if !crate::platform::recycle(&p) {
                toast(app, "Could not delete it (is it open in another app?)");
                return;
            }
            toast(app, format!("{} moved to the Recycle Bin", files::file_name(&p)));
            app.recents.borrow_mut().retain(|r| r != &p);
            crate::prefs::save_recents(&app.recents.borrow());
            crate::app::refresh_recents(app);
            viewer(app).set_dirty(false);
            match next.filter(|n| n.exists()) {
                Some(n) => open(app, &n),
                None => close(app),
            }
        }
        None => {
            toast(app, "Snip discarded");
            close(app);
        }
    }
}

/// Empties the picture tab and frees the picture, its edits and the filmstrip.
pub fn close(app: &Rc<App>) {
    {
        let mut s = app.view.borrow_mut();
        stop_anim(&mut s);
        s.generation += 1;
        s.path = None;
        s.pic = None;
        s.original = None;
        s.folder.clear();
        s.marks.clear();
        s.redo.clear();
        s.history.clear();
        s.future.clear();
        s.picked.clear();
    }
    viewer(app).set_comparing(false);
    viewer(app).set_file_type(SharedString::new());
    viewer(app).set_dims_text(SharedString::new());
    viewer(app).set_size_text(SharedString::new());
    viewer(app).set_can_share(false);
    refresh_undo(app);
    let v = viewer(app);
    v.set_animated(false);
    v.set_dirty(false);
    v.set_mode(SharedString::new());
    v.set_picture(slint::Image::default());
    v.set_info_name(SharedString::new());
    v.set_position(SharedString::new());
    v.set_has_prev(false);
    v.set_has_next(false);
    v.set_show_info(false);
    v.set_compare_images(ModelRc::default());
    v.set_thumbs(ModelRc::default());
    {
        let mut s = app.view.borrow_mut();
        s.thumbs = None;
        s.thumbs_dir = None;
        s.thumbs_gen += 1;
    }
    crate::app::update_title(app);
}

/// Before the picture goes away: true when there's nothing unsaved, or the person saved it or chose to discard it.
pub fn confirm_discard(app: &Rc<App>) -> bool {
    if !viewer(app).get_dirty() {
        return true;
    }
    let name = app.view.borrow().path.as_deref().map(files::file_name).unwrap_or_else(|| "this snip".into());
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Rimelight Media")
        .set_description(format!("Save the changes to {name}?"))
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom("Save".into(), "Don't save".into(), "Cancel".into()))
        .show();
    match answer {
        rfd::MessageDialogResult::Custom(c) if c == "Save" => {
            save(app);
            !viewer(app).get_dirty()
        }
        rfd::MessageDialogResult::Custom(c) if c == "Don't save" => true,
        rfd::MessageDialogResult::Yes => {
            save(app);
            !viewer(app).get_dirty()
        }
        rfd::MessageDialogResult::No => true,
        _ => false,
    }
}
fn print(app: &Rc<App>) {
    let Some(pic) = current(app) else { return };
    let title = app.view.borrow().path.as_deref().map(files::file_name).unwrap_or_else(|| "Picture".into());
    match crate::platform::print_image(crate::app::main_hwnd(app), &pic.rgba, pic.w, pic.h, &title) {
        Ok(true) => toast(app, "Sent to the printer"),
        Ok(false) => {}
        Err(e) => toast(app, format!("Could not print: {e}")),
    }
}

fn share(app: &Rc<App>) {
    let Some(path) = app.view.borrow().path.clone().filter(|p| p.exists()) else {
        toast(app, "Save the picture first, then share it.");
        return;
    };
    if viewer(app).get_dirty() {
        toast(app, "Sharing the saved file: changes not saved yet aren't included.");
    }
    if let Err(e) = crate::platform::share_file(crate::app::main_hwnd(app), &path, &files::file_name(&path)) {
        toast(app, format!("Could not share: {e}"));
    }
}

fn toggle_filmstrip(app: &Rc<App>) {
    let v = viewer(app);
    v.set_filmstrip(!v.get_filmstrip());
    sync_thumbs(app);
}

/// Keeps the filmstrip in step with the folder: new thumbnails (made in the background) for a new folder, otherwise
/// only the current and picked marks.
fn sync_thumbs(app: &Rc<App>) {
    if !viewer(app).get_filmstrip() {
        return;
    }
    let (folder, dir, same) = {
        let s = app.view.borrow();
        let dir = s.folder.first().and_then(|p| p.parent()).map(Path::to_path_buf);
        let same = s.thumbs.as_ref().is_some_and(|m| m.row_count() == s.folder.len()) && s.thumbs_dir == dir;
        (s.folder.clone(), dir, same)
    };
    if !same {
        let model = Rc::new(VecModel::from(
            folder
                .iter()
                .map(|p| ui::Thumb { image: slint::Image::default(), name: files::file_name(p).into(), current: false, selected: false })
                .collect::<Vec<_>>(),
        ));
        let generation = {
            let mut s = app.view.borrow_mut();
            s.thumbs_gen += 1;
            s.thumbs = Some(Rc::clone(&model));
            s.thumbs_dir = dir;
            s.picked.clear();
            s.thumbs_gen
        };
        viewer(app).set_thumbs(ModelRc::from(model));
        std::thread::spawn(move || {
            for (i, p) in folder.into_iter().enumerate() {
                let Some(pic) = picture::thumbnail(&p, 240, 176) else { continue };
                let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&pic.rgba, pic.w, pic.h);
                let keep_going = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                let flag = std::sync::Arc::clone(&keep_going);
                post(move |a| {
                    let s = a.view.borrow();
                    if s.thumbs_gen != generation {
                        flag.store(false, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                    if let Some(m) = s.thumbs.as_ref()
                        && let Some(mut row) = m.row_data(i)
                    {
                        row.image = slint::Image::from_rgba8(buf);
                        m.set_row_data(i, row);
                    }
                });
                if !keep_going.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
            }
        });
    }
    let s = app.view.borrow();
    if let Some(m) = s.thumbs.as_ref() {
        for i in 0..m.row_count() {
            if let Some(mut row) = m.row_data(i) {
                let (current, selected) = (i == s.index, s.picked.contains(&i));
                if row.current != current || row.selected != selected {
                    row.current = current;
                    row.selected = selected;
                    m.set_row_data(i, row);
                }
            }
        }
    }
}

/// A click in the filmstrip opens that picture; Ctrl+click (or Shift+click) picks pictures to compare side by side.
fn thumb_clicked(app: &Rc<App>, i: usize, add: bool) {
    let Some(path) = app.view.borrow().folder.get(i).cloned() else { return };
    if !add {
        if viewer(app).get_dirty() {
            toast(app, "Changes not saved: save them, or undo them, first.");
            return;
        }
        app.view.borrow_mut().picked.clear();
        viewer(app).set_comparing(false);
        open(app, &path);
        return;
    }
    let picked = {
        let mut s = app.view.borrow_mut();
        if s.picked.is_empty() {
            let current = s.index;
            s.picked.push(current);
        }
        if let Some(at) = s.picked.iter().position(|p| *p == i) {
            s.picked.remove(at);
        } else {
            s.picked.push(i);
        }
        s.picked.iter().filter_map(|p| s.folder.get(*p).cloned()).collect::<Vec<_>>()
    };
    sync_thumbs(app);
    let v = viewer(app);
    if picked.len() < 2 {
        v.set_comparing(false);
        return;
    }
    v.set_comparing(true);
    v.set_compare_images(ModelRc::new(VecModel::from(vec![slint::Image::default(); picked.len()])));
    let generation = app.view.borrow().generation;
    for (slot, p) in picked.into_iter().enumerate() {
        std::thread::spawn(move || {
            let Ok(pic) = picture::load(&p) else { return };
            // Side by side, each needs only a screen's worth of pixels.
            let pic = if pic.w.max(pic.h) > 2560 {
                let f = 2560.0 / pic.w.max(pic.h) as f32;
                picture::resize(&pic, (pic.w as f32 * f) as u32, (pic.h as f32 * f) as u32)
            } else {
                pic
            };
            let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&pic.rgba, pic.w, pic.h);
            post(move |a| {
                if a.view.borrow().generation != generation {
                    return;
                }
                let model = viewer(a).get_compare_images();
                if slot < model.row_count() {
                    model.set_row_data(slot, slint::Image::from_rgba8(buf));
                }
            });
        });
    }
}

pub fn close_compare(app: &Rc<App>) {
    app.view.borrow_mut().picked.clear();
    viewer(app).set_comparing(false);
    sync_thumbs(app);
}