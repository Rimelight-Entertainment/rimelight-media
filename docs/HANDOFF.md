# Rimelight Media: handoff (as of v0.1.8, 2026-11-21)

Rust + Slint 1.18 Windows app replacing Photos, Media Player/VLC, Notepad, Snipping Tool, Camera, Sound Recorder and
Paint. See README.md for features, RELEASING.md for releases (push a `vX.Y.Z` tag; GitHub Actions builds the Velopack
installer, about 11 minutes).

## How the owner works

- Haiku subagents (low effort) do the implementation. The main session orchestrates, builds and fixes. Agents never run
  cargo; the main session builds once per batch.
- Give each parallel agent its own files, and tell them to re-read before editing shared ones (ui/state.slint,
  ui/widgets.slint, ui/app.slint, src/app.rs).
- Check UI with `target/debug/rimelight-media.exe --screenshot <png> [--page image|audio|video|text|voice|camera] [file]`
  and APPDATA set to a temp folder, so the user's running copy is untouched. It's non-interactive and never takes
  focus. Never send key presses or clicks to the user's desktop without asking.
- The owner is very picky about UI consistency: left/right split toolbars (never centred), confirm buttons flush right,
  everything vertically centred, consistent gaps, no stray or trailing dividers, and in-app dialogs only (never native
  message boxes).
- Commit and push only when asked. Release = bump Cargo.toml version, commit, tag, push.

## Architecture pointers

- ui/widgets.slint: the shared components. Use them everywhere:
  - ToolBar (left/right split via a stretch spacer)
  - ActionRow (edit rows, confirm buttons flush right)
  - StatusBar (left-text, or leave it empty and lay out the whole bar in children)
  - ToolDivider (only between groups)
  - EmptyState (icon, title, supported types, Open + New, recents)
  - IconButton (tooltip is a PopupWindow)
  - ConfirmDialog
- Questions: `app::ask(app, title, name, detail, primary, secondary, |app, choice| ...)` (0 primary, 1 secondary, 2 Cancel),
  routed through textctl::answer to app::take_ask_answer. Also viewer::settle_picture, viewer::settle_canvas, textctl::settle.
- Tray hand-off (src/app.rs show_main/handoff/finish_close, src/main.rs):
  - `--background` runs on the software renderer (about 22 MB; GPU is about 62 MB even with no window).
  - Showing the window spawns a GPU copy with `--takeover --page X [file]` and exits.
  - Closing to the tray spawns `--background --takeover --after <pid>`; the new copy waits for the old one, which
    releases the start lock and exits via TerminateProcess (platform::exit_now).
  - It stays put if a recording, unsaved text or a canvas is open.
  - Single instance: a named mutex (platform::take_start_lock) plus the instance.txt TCP forward. app::queue_open waits
    until files stop arriving (400 ms) and opens them as one set.
- Win+Shift+S (src/platform.rs, src/hotkeys.rs):
  - A WH_KEYBOARD_LL hook on its own TIME_CRITICAL thread swallows Win+Shift+S. It re-installs itself when the
    foreground window changes (at most once per 2 s), ignores injected keys and cross-checks GetAsyncKeyState.
  - Backstop: a WinEvent hook (show/foreground) kills ScreenClippingHost.exe if Windows' snip opens anyway and snips here.
  - Failures and backstop kills go to %APPDATA%\Rimelight Media\hook-log.txt (trimmed at 200 KB).
  - Known Windows limit (owner accepted): with an elevated window in front (Task Manager), UIPI hides keys from us. Print
    Screen does nothing there; Win+Shift+S works only through the backstop, and the overlay may not get focus.
- Window animations: platform::start_no_window_animations installs a WH_CALLWNDPROC hook on the UI thread that sets
  DWMWA_TRANSITIONS_FORCEDISABLED just before any "RimelightMediaWindow" (class name set in main.rs) is first shown. The
  snip overlay is created per snip and dropped after (no hidden spare window).
- Video: frames are scaled to the window, queue of 5 (player.rs MAX_QUEUED_FRAMES, the owner says it is NOT the memory
  source: leave it). Player::release joins the workers (close/rename only, never on seek).
- Memory (measured): ~44 MB of each window copy is the NVIDIA OpenGL driver (femtovg). The app's own heap is ~7 MB at
  rest. Proposed, not done: software renderer for non-GPU pages via the hand-off; SVGs are drawn at >= 2048 px
  (picture.rs SVG_MIN_SIDE), about 16 MB each. FFmpeg DLLs are delay-loaded (build.rs).
- Pictures: image crate, resvg (SVG), FFmpeg for HEIC/AVIF (picture_heif.rs), rawler for RAW (picture_raw.rs). Files are
  read into memory (no open handles, so rename works). One copy per picture; Revert uses the oldest undo step; undo is
  capped at 256 MB. Canvas/markup in viewer.rs/picture.rs; Move (tool 11, hand icon) selects/moves/resizes marks with 8
  handles; middle-drag pans everywhere.
- Text: textctl.rs; syntect + two-face highlighting off the UI thread, drawn as per-colour monospace layers under a
  transparent TextInput (code files don't wrap; Markdown isn't coloured). Highlighter unloads after 30 s idle. Plain text
  has a Mono/Sans/Serif choice (prefs text_font).
- File types: one ProgID per extension (install.rs); old per-category ProgIDs are kept because existing defaults point at
  them; scripts (bat/ps1/sh) are Open-with only. Category .ico files are rendered with the in-app glyphs.
- samples/: one file per supported type (text by hand, media from make-samples.ps1). samples/raw/ (camera RAW, HEIC,
  233 MB, from get-raw-samples.ps1) is stored with Git LFS.
- Our FFmpeg build: tools/ffmpeg/options.txt and build.sh (a separate release asset, pinned in tools/get-ffmpeg.ps1).

## Open or unverified (as of v0.1.8)

- Video seek: "Opening..." no longer shows on seek; the owner reported seeks felt choppier, cause not found.
- Unverified: italic in the markdown preview, camera with a real webcam, cast (DLNA) with a real TV, MP3 voice format.
- Known gaps: dropping an image onto the window during markup places it as a mark; .hif has no sample; sound output
  rebuild is skipped if the new device has a different sample rate.
- Possible size work: trim old decoders from our FFmpeg (cinepak, svq3, wmv1/2, msmpeg4, theora, prores).
