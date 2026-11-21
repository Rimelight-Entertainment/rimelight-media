//! The Text tab (Notepad's job): plain text files with their encoding (UTF-8, with or without BOM, UTF-16) and line endings
//! kept on save; Markdown as flat blocks for the native preview; and the JSON tools (format, minify, validate, sort keys,
//! the collapsible tree). The editor (ui/text_page.slint) holds the text; Rust reads it when it needs it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::Serialize as _;
use serde_json::Value;
use slint::{ComponentHandle as _, ModelRc, SharedString, VecModel};

use crate::app::{self, App, toast, with_app};
use crate::files;
use crate::ui;

/// The largest file the tab opens (a bigger one is a log for another tool).
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Enc {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

impl Enc {
    fn label(self) -> &'static str {
        match self {
            Enc::Utf8 => "UTF-8",
            Enc::Utf8Bom => "UTF-8 BOM",
            Enc::Utf16Le => "UTF-16 LE",
            Enc::Utf16Be => "UTF-16 BE",
        }
    }
}

/// The open text file and the tab's state. The editor holds the text itself (TextDoc.text), always with LF line ends.
#[derive(Default)]
pub struct TextState {
    pub path: Option<PathBuf>,
    enc: Enc,
    /// The file used CRLF line ends; saving writes them back.
    crlf: bool,
    /// The caret as a byte offset into the text (the editor reports it), and the last selection the find tools made.
    caret: usize,
    selection: Option<(usize, usize)>,
    /// JSON tree: the paths of the containers folded shut.
    collapsed: HashSet<String>,
    /// The action waiting for the answer to "Save changes to ...?".
    pending: Option<Pending>,
}

/// What to do once the unsaved-changes question is answered (Save, or Don't save; Cancel does nothing).
pub enum Pending {
    Open(PathBuf),
    New,
    Close,
    /// The window is closing: finish the close (app::finish_close).
    Quit,
}

fn doc(app: &Rc<App>) -> ui::TextDoc {
    app.ui.global::<ui::TextDoc>()
}

fn text_of(app: &Rc<App>) -> String {
    doc(app).get_text().to_string()
}

/// Connects the editor's callbacks (called once at start-up).
pub fn wire(app: &Rc<App>) {
    let d = doc(app);
    d.on_caret_moved(|i| {
        with_app(|a| {
            a.text.borrow_mut().caret = i.max(0) as usize;
            update_status(a);
        })
    });
    d.on_edited(|| with_app(edited));
    d.on_save(|| {
        with_app(|a| {
            save(a, false);
        })
    });
    d.on_save_as(|| {
        with_app(|a| {
            save(a, true);
        })
    });
    d.on_new_doc(|| with_app(new_doc));
    d.on_format_json(|n| with_app(|a| format_json(a, n)));
    d.on_minify_json(|| with_app(minify_json));
    d.on_validate_json(|| with_app(validate_json));
    d.on_sort_json(|| with_app(sort_json));
    d.on_toggle_node(|p: SharedString| with_app(|a| toggle_node(a, &p)));
    d.on_find_next(|needle: SharedString| with_app(|a| find_next(a, &needle)));
    d.on_replace_one(|needle: SharedString, repl: SharedString| with_app(|a| replace_one(a, &needle, &repl)));
    d.on_replace_all(|needle: SharedString, repl: SharedString| with_app(|a| replace_all(a, &needle, &repl)));
    d.on_revert(|| with_app(revert));
    app.ui.global::<ui::Shell>().on_confirm_answer(|choice| with_app(|a| answer(a, choice)));
}

/// The window's title part for this tab: the file's name (the editor shows the unsaved marker).
pub fn title(app: &Rc<App>) -> String {
    app.text.borrow().path.as_deref().map(files::file_name).unwrap_or_else(|| "Untitled".into())
}

/// Opens a text file in the tab (asks about unsaved changes first; the file is read once the answer is in).
pub fn open(app: &Rc<App>, path: &Path) {
    if path.metadata().map(|m| m.len()).unwrap_or(0) > MAX_BYTES {
        toast(app, "That file is too big for the Text tab.");
        return;
    }
    if !settle(app, Pending::Open(path.to_path_buf())) {
        return;
    }
    load(app, path);
}

/// Reads a file into the tab (no question asked: the caller has dealt with unsaved changes).
fn load(app: &Rc<App>, path: &Path) {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            toast(app, format!("Could not read {}: {e}", files::file_name(path)));
            return;
        }
    };
    let (enc, text) = decode(&bytes);
    let crlf = text.contains("\r\n");
    {
        let mut st = app.text.borrow_mut();
        *st = TextState { path: Some(path.to_path_buf()), enc, crlf, ..TextState::default() };
    }
    set_meta(app, path);
    let d = doc(app);
    d.set_text(text.replace("\r\n", "\n").into());
    d.set_dirty(false);
    d.set_view(if d.get_markdown() { 1 } else { 0 });
    app::go(app, "text");
    refresh(app);
    update_status(app);
    app::update_title(app);
}

/// An empty, untitled document (asks about unsaved changes first).
fn new_doc(app: &Rc<App>) {
    if settle(app, Pending::New) {
        fresh(app);
    }
}

/// Makes the tab an empty, untitled document (no question asked).
fn fresh(app: &Rc<App>) {
    clear(app);
    doc(app).set_has_doc(true);
    app::go(app, "text");
    refresh(app);
    update_status(app);
    app::update_title(app);
}

/// Empties the tab (no question asked: the caller has already dealt with unsaved changes).
pub fn clear(app: &Rc<App>) {
    *app.text.borrow_mut() = TextState::default();
    let d = doc(app);
    d.set_text("".into());
    d.set_has_doc(false);
    d.set_dirty(false);
    d.set_file_name("".into());
    d.set_markdown(false);
    d.set_json(false);
    d.set_view(0);
    d.set_find_status("".into());
    d.set_message("".into());
    d.set_status("".into());
}

/// The X in the top bar: back to the empty tab (asking first about unsaved changes).
pub fn close(app: &Rc<App>) {
    if settle(app, Pending::Close) {
        clear(app);
    }
}

/// True when the text tab has unsaved changes.
pub fn is_dirty(app: &Rc<App>) -> bool {
    doc(app).get_dirty()
}

/// Returns true when the action may run now (no unsaved changes). Otherwise the in-app question is shown, the action is
/// kept, and `answer` runs it (or not) later; this returns false.
pub fn settle(app: &Rc<App>, then: Pending) -> bool {
    if !is_dirty(app) {
        return true;
    }
    app.text.borrow_mut().pending = Some(then);
    let shell = app.ui.global::<ui::Shell>();
    shell.set_confirm_name(title(app).into());
    shell.set_confirm_open(true);
    false
}

/// The question's answer: 0 Save (then the action, if the save worked), 1 Don't save (the action, dropping the changes),
/// 2 Cancel (nothing).
fn answer(app: &Rc<App>, choice: i32) {
    // The same dialog asks about an unsaved canvas (the picture page) when one is waiting for the answer.
    if crate::viewer::has_pending(app) {
        crate::viewer::answer(app, choice);
        return;
    }
    app.ui.global::<ui::Shell>().set_confirm_open(false);
    let pending = app.text.borrow_mut().pending.take();
    let Some(then) = pending else { return };
    let go_on = match choice {
        0 => save(app, false),
        1 => true,
        _ => false,
    };
    if go_on {
        run(app, then);
    }
}

fn run(app: &Rc<App>, then: Pending) {
    match then {
        Pending::Open(path) => load(app, &path),
        Pending::New => fresh(app),
        Pending::Close => {
            clear(app);
            app::update_title(app);
        }
        Pending::Quit => app::finish_close(app),
    }
}

/// Discard changes: the file is read again from disk and the modified flag cleared (an untitled text is emptied).
pub fn revert(app: &Rc<App>) {
    let path = app.text.borrow().path.clone();
    match path {
        Some(p) => {
            let view = doc(app).get_view();
            load(app, &p);
            doc(app).set_view(view);
            toast(app, "Changes discarded");
        }
        None => {
            doc(app).set_text("".into());
            doc(app).set_dirty(false);
            refresh(app);
            update_status(app);
        }
    }
}

/// Writes the text (Save, or Save as when there is no file yet or `as_new`). Returns false when nothing was saved.
pub fn save(app: &Rc<App>, as_new: bool) -> bool {
    let current = app.text.borrow().path.clone();
    let path = match current {
        Some(p) if !as_new => p,
        current => {
            let name = current.as_deref().map(files::file_name).unwrap_or_else(|| "Untitled.txt".into());
            let picked = rfd::FileDialog::new()
                .set_title("Save as")
                .set_file_name(name)
                .add_filter("Text", files::TEXT_EXTS)
                .add_filter("Everything", &["*"])
                .save_file();
            match picked {
                Some(p) => p,
                None => return false,
            }
        }
    };
    let (enc, crlf) = {
        let st = app.text.borrow();
        (st.enc, st.crlf)
    };
    let mut text = text_of(app);
    if crlf {
        text = text.replace('\n', "\r\n");
    }
    if let Err(e) = std::fs::write(&path, encode(enc, &text)) {
        toast(app, format!("Could not save {}: {e}", files::file_name(&path)));
        return false;
    }
    app.text.borrow_mut().path = Some(path.clone());
    set_meta(app, &path);
    doc(app).set_dirty(false);
    app::remember(app, &path);
    app::update_title(app);
    update_status(app);
    toast(app, format!("Saved {}", files::file_name(&path)));
    true
}

fn set_meta(app: &Rc<App>, path: &Path) {
    let ext = files::ext_of(path);
    let d = doc(app);
    d.set_has_doc(true);
    d.set_file_name(files::file_name(path).into());
    d.set_markdown(matches!(ext.as_str(), "md" | "markdown"));
    d.set_json(ext == "json");
}

fn edited(app: &Rc<App>) {
    app.text.borrow_mut().selection = None;
    doc(app).set_dirty(true);
    update_status(app);
    refresh(app);
    app::update_title(app);
}

/// Ln, Col, characters, line ends and encoding, separated by " | ".
fn update_status(app: &Rc<App>) {
    let text = text_of(app);
    let status = {
        let st = app.text.borrow();
        let mut c = st.caret.min(text.len());
        while !text.is_char_boundary(c) {
            c -= 1;
        }
        let before = &text[..c];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        let eol = if st.crlf { "CRLF" } else { "LF" };
        format!("Ln {line}, Col {col} | {} characters | {eol} | {}", text.chars().count(), st.enc.label())
    };
    doc(app).set_status(status.into());
}

/// Rebuilds the preview (Markdown) or the tree (JSON) after a change.
fn refresh(app: &Rc<App>) {
    let d = doc(app);
    let text = text_of(app);
    if d.get_markdown() {
        d.set_md_blocks(ModelRc::new(VecModel::from(markdown_blocks(&text))));
    }
    if d.get_json() {
        let collapsed = app.text.borrow().collapsed.clone();
        let mut rows = Vec::new();
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            walk(&v, "", "root".into(), 0, &collapsed, &mut rows);
        }
        d.set_json_rows(ModelRc::new(VecModel::from(rows)));
    }
}

fn set_text(app: &Rc<App>, text: &str) {
    let d = doc(app);
    d.set_text(text.into());
    d.set_dirty(true);
    app.text.borrow_mut().selection = None;
    update_status(app);
    refresh(app);
    app::update_title(app);
}

/// Moves the caret and selects `start..end` in the editor (the editor applies it).
fn select(app: &Rc<App>, start: usize, end: usize) {
    app.text.borrow_mut().selection = Some((start, end));
    let d = doc(app);
    d.set_sel_start(start as i32);
    d.set_sel_end(end as i32);
    d.set_sel_serial(d.get_sel_serial() + 1);
}

// --- Encoding -------------------------------------------------------------------------------------------------------

/// The text and its encoding, from the bytes (BOM first; without one, UTF-8, invalid bytes shown as �).
fn decode(bytes: &[u8]) -> (Enc, String) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (Enc::Utf8Bom, String::from_utf8_lossy(rest).into_owned());
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return (Enc::Utf16Le, utf16(rest, u16::from_le_bytes));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return (Enc::Utf16Be, utf16(rest, u16::from_be_bytes));
    }
    (Enc::Utf8, String::from_utf8_lossy(bytes).into_owned())
}

fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

fn encode(enc: Enc, text: &str) -> Vec<u8> {
    match enc {
        Enc::Utf8 => text.as_bytes().to_vec(),
        Enc::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        Enc::Utf16Le => {
            let mut out = vec![0xFF, 0xFE];
            out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            out
        }
        Enc::Utf16Be => {
            let mut out = vec![0xFE, 0xFF];
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            out
        }
    }
}

// --- Find and replace ------------------------------------------------------------------------------------------------

/// The first match at or after `from` (ASCII letters match in either case; the offsets are byte offsets).
fn find_bytes(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() || from > h.len() || n.len() > h.len() - from {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

fn count_matches(hay: &str, needle: &str) -> usize {
    let mut count = 0;
    let mut pos = 0;
    while let Some(i) = find_bytes(hay, needle, pos) {
        count += 1;
        pos = i + needle.len();
    }
    count
}

fn find_next(app: &Rc<App>, needle: &str) {
    let d = doc(app);
    if needle.is_empty() {
        d.set_find_status("".into());
        return;
    }
    let text = text_of(app);
    let count = count_matches(&text, needle);
    d.set_find_status(if count == 0 { "No matches".to_string() } else { format!("{count} matches") }.into());
    let from = app.text.borrow().caret.min(text.len());
    if let Some(i) = find_bytes(&text, needle, from).or_else(|| find_bytes(&text, needle, 0)) {
        select(app, i, i + needle.len());
    }
}

fn replace_one(app: &Rc<App>, needle: &str, repl: &str) {
    if needle.is_empty() {
        return;
    }
    let text = text_of(app);
    // The match the find tools selected comes first; otherwise the next one after the caret.
    let selected = app.text.borrow().selection;
    let hit = match selected {
        Some((s, e)) if s < e && text.get(s..e).is_some_and(|t| t.eq_ignore_ascii_case(needle)) => Some(s),
        _ => {
            let from = app.text.borrow().caret.min(text.len());
            find_bytes(&text, needle, from).or_else(|| find_bytes(&text, needle, 0))
        }
    };
    let Some(i) = hit else {
        doc(app).set_find_status("No matches".into());
        return;
    };
    let out = format!("{}{repl}{}", &text[..i], &text[i + needle.len()..]);
    set_text(app, &out);
    select(app, i, i + repl.len());
}

fn replace_all(app: &Rc<App>, needle: &str, repl: &str) {
    if needle.is_empty() {
        return;
    }
    let text = text_of(app);
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    let mut n = 0;
    while let Some(i) = find_bytes(&text, needle, pos) {
        out.push_str(&text[pos..i]);
        out.push_str(repl);
        pos = i + needle.len();
        n += 1;
    }
    if n == 0 {
        doc(app).set_find_status("No matches".into());
        return;
    }
    out.push_str(&text[pos..]);
    set_text(app, &out);
    doc(app).set_find_status(format!("Replaced {n}").into());
}

// --- Markdown ---------------------------------------------------------------------------------------------------------

/// Collects the Markdown into flat blocks: kind 0 heading (level), 1 paragraph, 2 list item (indent, check: -1 none,
/// 0 open task, 1 done task), 3 code line, 4 quote, 5 table row (level 1 for the header), 6 rule.
#[derive(Default)]
struct Blocks {
    out: Vec<ui::MdBlock>,
    buf: String,
    /// Inside a list item: its indent and task state.
    item: Option<(i32, i32)>,
    lists: i32,
    quote: i32,
    cells: Vec<String>,
}

impl Blocks {
    fn emit(&mut self, kind: i32, level: i32, indent: i32, check: i32, text: String) {
        if text.is_empty() && kind != 6 {
            return;
        }
        self.out.push(ui::MdBlock { kind, level, indent, check, text: text.into() });
    }

    /// Ends the text collected so far as a paragraph, list item or quote line.
    fn flush(&mut self) {
        let text = std::mem::take(&mut self.buf).trim().to_string();
        if let Some((indent, check)) = self.item {
            self.emit(2, 0, indent, check, text);
        } else if self.quote > 0 {
            self.emit(4, 0, 0, -1, text);
        } else {
            self.emit(1, 0, 0, -1, text);
        }
    }

    fn row(&mut self, level: i32) {
        let text = self.cells.join("   |   ");
        self.cells.clear();
        self.emit(5, level, 0, -1, text);
    }
}

pub fn markdown_blocks(src: &str) -> Vec<ui::MdBlock> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let mut b = Blocks::default();
    let mut heading = 0;
    let mut head = false;
    for ev in Parser::new_ext(src, opts) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                b.flush();
                heading = level as i32;
            }
            Event::End(TagEnd::Heading(_)) => {
                let text = std::mem::take(&mut b.buf).trim().to_string();
                b.emit(0, heading, 0, -1, text);
            }
            Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => b.flush(),
            Event::Start(Tag::List(_)) => {
                b.flush();
                b.lists += 1;
            }
            Event::End(TagEnd::List(_)) => {
                b.flush();
                b.lists -= 1;
            }
            Event::Start(Tag::Item) => {
                b.flush();
                b.item = Some(((b.lists - 1).max(0), -1));
            }
            Event::End(TagEnd::Item) => {
                b.flush();
                b.item = None;
            }
            Event::TaskListMarker(done) => {
                if let Some(item) = b.item.as_mut() {
                    item.1 = i32::from(done);
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                b.flush();
                b.quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                b.flush();
                b.quote -= 1;
            }
            Event::Start(Tag::CodeBlock(_)) => b.flush(),
            Event::End(TagEnd::CodeBlock) => {
                let text = std::mem::take(&mut b.buf);
                for line in text.trim_end_matches('\n').split('\n') {
                    b.out.push(ui::MdBlock { kind: 3, level: 0, indent: 0, check: -1, text: line.into() });
                }
            }
            Event::Start(Tag::Table(_)) => b.flush(),
            Event::Start(Tag::TableHead) => {
                b.flush();
                head = true;
            }
            Event::End(TagEnd::TableHead) => {
                b.row(1);
                head = false;
            }
            Event::Start(Tag::TableRow) => b.flush(),
            Event::End(TagEnd::TableRow) => b.row(i32::from(head)),
            Event::Start(Tag::TableCell) => b.buf.clear(),
            Event::End(TagEnd::TableCell) => {
                let cell = std::mem::take(&mut b.buf).trim().to_string();
                b.cells.push(cell);
            }
            Event::Rule => {
                b.flush();
                b.emit(6, 0, 0, -1, String::new());
            }
            Event::Text(t) | Event::Code(t) => b.buf.push_str(&t),
            Event::SoftBreak | Event::HardBreak => b.buf.push(' '),
            // Emphasis, links and other inline tags show their text only (the blocks are styled per block).
            _ => {}
        }
    }
    b.flush();
    b.out
}

// --- JSON -------------------------------------------------------------------------------------------------------------

fn parse_json(app: &Rc<App>, text: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => Some(v),
        Err(e) => {
            let msg = e.to_string();
            let msg = msg.split(" at line ").next().unwrap_or_default().to_string();
            doc(app).set_message(format!("Not valid JSON: line {}, column {}: {msg}", e.line(), e.column()).into());
            let off = offset_of(text, e.line(), e.column());
            select(app, off, off);
            None
        }
    }
}

/// The byte offset of a 1-based line and column.
fn offset_of(text: &str, line: usize, col: usize) -> usize {
    let mut off = 0;
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i + 1 == line {
            let mut o = (off + col.saturating_sub(1)).min(off + l.len());
            while !text.is_char_boundary(o) {
                o -= 1;
            }
            return o;
        }
        off += l.len();
    }
    text.len()
}

fn pretty(v: &Value, indent: &[u8]) -> String {
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, serde_json::ser::PrettyFormatter::with_indent(indent));
    let _ = v.serialize(&mut ser);
    String::from_utf8(buf).unwrap_or_default()
}

fn format_json(app: &Rc<App>, indent: i32) {
    let text = text_of(app);
    let Some(v) = parse_json(app, &text) else { return };
    let spaces = if indent == 4 { "    " } else { "  " };
    set_text(app, &pretty(&v, spaces.as_bytes()));
    doc(app).set_message(format!("Formatted with {} spaces", spaces.len()).into());
}

fn minify_json(app: &Rc<App>) {
    let text = text_of(app);
    let Some(v) = parse_json(app, &text) else { return };
    let out = serde_json::to_string(&v).unwrap_or_default();
    let saved = text.chars().count().saturating_sub(out.chars().count());
    set_text(app, &out);
    doc(app).set_message(format!("Minified ({saved} characters less)").into());
}

fn validate_json(app: &Rc<App>) {
    let text = text_of(app);
    let Some(v) = parse_json(app, &text) else { return };
    let what = match &v {
        Value::Object(m) => format!("{} keys", m.len()),
        Value::Array(a) => format!("{} items", a.len()),
        _ => "one value".to_string(),
    };
    doc(app).set_message(format!("Valid JSON: {what}").into());
}

fn sort_json(app: &Rc<App>) {
    let text = text_of(app);
    let Some(mut v) = parse_json(app, &text) else { return };
    sort_keys(&mut v);
    set_text(app, &pretty(&v, b"  "));
    doc(app).set_message("Keys sorted (A to Z, every level)".into());
}

/// Puts every object's keys in order, at every level.
fn sort_keys(v: &mut Value) {
    match v {
        Value::Object(m) => {
            let mut entries: Vec<(String, Value)> = std::mem::take(m).into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, child) in entries.iter_mut() {
                sort_keys(child);
            }
            *m = entries.into_iter().collect();
        }
        Value::Array(a) => a.iter_mut().for_each(sort_keys),
        _ => {}
    }
}

fn toggle_node(app: &Rc<App>, path: &str) {
    {
        let mut st = app.text.borrow_mut();
        if !st.collapsed.remove(path) {
            st.collapsed.insert(path.to_string());
        }
    }
    refresh(app);
}

/// One row of the tree: its depth, key, value summary, and whether it folds (containers with items).
fn walk(v: &Value, path: &str, label: String, depth: i32, collapsed: &HashSet<String>, rows: &mut Vec<ui::JsonRow>) {
    let (container, value) = match v {
        Value::Object(m) if !m.is_empty() => (true, format!("{} keys", m.len())),
        Value::Array(a) if !a.is_empty() => (true, format!("{} items", a.len())),
        Value::Object(_) => (false, "{}".to_string()),
        Value::Array(_) => (false, "[]".to_string()),
        scalar => {
            let text = serde_json::to_string(scalar).unwrap_or_default();
            let short: String = text.chars().take(160).collect();
            (false, if short.len() < text.len() { format!("{short}…") } else { short })
        }
    };
    let open = !collapsed.contains(path);
    rows.push(ui::JsonRow { depth, label: label.into(), value: value.into(), container, open, path: path.into() });
    if container && open {
        match v {
            Value::Object(m) => {
                for (k, child) in m {
                    walk(child, &format!("{path}/{k}"), k.to_string(), depth + 1, collapsed, rows);
                }
            }
            Value::Array(a) => {
                for (i, child) in a.iter().enumerate() {
                    walk(child, &format!("{path}/{i}"), format!("[{i}]"), depth + 1, collapsed, rows);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_round_trip() {
        let text = "héllo\nwörld";
        for enc in [Enc::Utf8, Enc::Utf8Bom, Enc::Utf16Le, Enc::Utf16Be] {
            let (back_enc, back) = decode(&encode(enc, text));
            assert_eq!(back_enc, enc);
            assert_eq!(back, text);
        }
    }

    #[test]
    fn finds_ignoring_ascii_case() {
        assert_eq!(find_bytes("Hello hello", "HELLO", 1), Some(6));
        assert_eq!(count_matches("aXa xa", "A"), 3);
        assert_eq!(offset_of("ab\ncd", 2, 2), 4);
    }

    #[test]
    fn markdown_blocks_cover_the_common_forms() {
        let md = "# Title\n\nSome *text*.\n\n- [x] done\n- open\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```\ncode\n```\n\n> quote\n\n---\n";
        let blocks = markdown_blocks(md);
        let kinds: Vec<i32> = blocks.iter().map(|b| b.kind).collect();
        assert_eq!(kinds, vec![0, 1, 2, 2, 5, 5, 3, 4, 6]);
        assert_eq!(blocks[2].check, 1);
        assert_eq!(blocks[3].check, -1);
    }

    #[test]
    fn sorts_keys_at_every_level() {
        let mut v: Value = serde_json::from_str(r#"{"b":1,"a":{"d":2,"c":[{"z":0,"y":0}]}}"#).unwrap();
        sort_keys(&mut v);
        assert_eq!(serde_json::to_string(&v).unwrap(), r#"{"a":{"c":[{"y":0,"z":0}],"d":2},"b":1}"#);
    }
}
