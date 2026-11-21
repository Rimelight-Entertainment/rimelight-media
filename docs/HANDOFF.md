# Rimelight Media: handoff (as of v0.1.7, 2026-11-21)

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
  - Closing to the tray spawns `--background --takeover` and exits.
  - It stays put if a recording, unsaved text or a canvas is open.
  - Single instance: a named mutex (platform::take_start_lock) plus the instance.txt TCP forward. app::queue_open waits
    until files stop arriving (400 ms) and opens them as one set.
- Win+Shift+S (src/platform.rs, src/hotkeys.rs):
  - A WH_KEYBOARD_LL hook tracks Win and Shift itself and is re-installed every 30 s.
  - It logs to %APPDATA%\Rimelight Media\hook-log.txt.
  - Backstop: every 250 ms, platform::close_windows_snip kills ScreenClippingHost.exe and snips instead.
  - Finding so far: failed presses never reach our hook (possibly Discord's or Steam's hooks run first). Unconfirmed.
- Snip overlay is created once and reused (capturectl SPARE_OVERLAY, prewarm_overlay), and its selection is reset on each
  show.
- Video: frames are scaled to the window, queue of 5 (player.rs MAX_QUEUED_FRAMES), decoded straight into a
  SharedPixelBuffer.
- Pictures: image crate, resvg (SVG), FFmpeg for HEIC/AVIF (picture_heif.rs, tile grids), rawler for RAW
  (picture_raw.rs, about 3 MB of exe). Canvas and markup live in viewer.rs/picture.rs (marks, burn_marks, flood fill,
  pixel eraser kind 8).
- Text: textctl.rs (pulldown-cmark into MdBlock with StyledText, plus the JSON tools); text_page.slint.
- Our FFmpeg build: tools/ffmpeg/options.txt and build.sh (a separate release asset, pinned in tools/get-ffmpeg.ps1).
  Its codec list can be changed.

## Open or unverified (v0.1.7 has the round-3 fixes, but the user hasn't tested them yet)

- The user is about to send a new list of points after testing 0.1.7.
- Unverified in the app: tray hand-off delay (if it feels too slow, go back to GPU-only and trim elsewhere), the
  ScreenClippingHost backstop, italic in the markdown preview (font needs an italic face), the pixel eraser's live
  preview, waveform seek and CPU, new-text focus, the camera with a real webcam, cast (DLNA) with a real TV, MP3 voice
  format (encoder may be missing from our FFmpeg).
- Known gaps: marker click-to-seek isn't implemented; dropping an image onto the window during markup places it as a
  mark instead of opening it; Win+Shift+S misses aren't root-caused.
- Possible size work: trim old decoders from our FFmpeg (cinepak, svq3, wmv1/2, msmpeg4, theora, prores) for maybe
  1-2 MB.
