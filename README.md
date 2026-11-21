# rimelight-media

One small Windows app for pictures, video, music, text, screenshots, recording, the camera and quick drawings. About
30 MB installed, it replaces these Windows and third-party apps:

| Replaces | With |
|---|---|
| Photos | the Image tab |
| Media Player, Movies & TV, VLC | the Audio and Video tabs |
| Notepad | the Text tab |
| Snipping Tool | Print Screen / Win+Shift+S snips and screen recording |
| Camera | the camera page (top right) |
| Sound Recorder | the voice recorder (top right) |
| Paint | the canvas (top right) |

Rust + Slint, with our own trimmed LGPL FFmpeg build for video and sound.

## Install

Download `RimelightMedia-win-Setup.exe` from the [latest release](https://github.com/Rimelight-Entertainment/rimelight-media/releases/latest)
and run it. It installs for your user only (no admin), adds a Start menu entry, and updates itself when a new release
comes out. Then open **Settings > Install, updates and default apps > Open Windows default apps** to make it the default
for pictures, video, music and text (Settings lists any file type that still opens with another app). Uninstall from
Windows Settings > Apps > Installed apps.

## What it does

- **Pictures:** PNG, JPEG, WebP, SVG, GIF, BMP, ICO, HEIC/HEIF, AVIF and camera RAW (DNG, CR2/CR3, NEF, ARW, RAF, ORF,
  RW2 and more; view only, save as PNG or JPEG). Zoom and pan, browse the folder or just the files you opened together,
  check pictures to compare them side by side in a grid, animated GIF/WebP with play, pause and frame stepping, details
  panel, print, share, copy, text recognition, rename from the title bar.
  Editing: crop, rotate, flip, resize with drag handles, markup (pen and highlighter with a thickness slider, arrow,
  line, box, ellipse, filled shapes, text, eraser, eyedropper, paint bucket), paste pictures in, undo/redo, discard.
- **Canvas (Paint):** a blank canvas (window size, 1080p, A4 or custom; white or transparent) with the same tools,
  plus rectangle select (copy, cut, crop, delete) and move. Paste or drop a picture to draw on it. Saves to
  Pictures\Canvas.
- **Video and music:** MP4, MKV, WebM, MOV, AVI, MPEG and more (H.264, HEVC, AV1, VP9 and others, no Windows codec
  extensions needed); MP3, FLAC, WAV, OGG, Opus, M4A and more. Audio track choice, subtitles (embedded text and picture
  subtitles, or SRT/VTT/ASS files) with delay and size, playback speed slider 0.25x to 3x, repeat, frame snapshots,
  trimming with drag handles over the waveform, waveform seek bar, status bar with codec details, cast to DLNA TVs and
  speakers on your network.
- **Text (Notepad):** TXT, Markdown, JSON, TOML, YAML, INI, CSV, XML, logs and more. Markdown with edit, split and
  preview views; JSON format, minify, validate (with the error's line and column), sort keys and a tree view; find and
  replace, word wrap; keeps the file's encoding (UTF-8, UTF-8 BOM, UTF-16) and line endings; Notepad-style status bar.
- **Capture:** Print Screen or Win+Shift+S snips an area, a window or the whole screen, including Rimelight Media itself
  (copied, saved and opened for markup). Screen recording to MP4 with the computer's sound and/or microphone.
- **Camera:** photos and video from any webcam, timer, grid, mirror, resolution; saves to Pictures\Camera Roll.
- **Voice recorder:** microphone choice, pause/resume, markers, M4A/WAV/FLAC, and a list of recordings to play,
  rename, delete or show in the folder.

## Keys

| Where | Keys |
|---|---|
| Anywhere | Print Screen or Win+Shift+S snip · Alt+Print Screen window · Ctrl+Print Screen whole screen · Shift+Print Screen record · Ctrl+O open · Ctrl+N new canvas · F11 full screen |
| Pictures | Left/Right previous/next · + / - / 0 / 1 zoom · R rotate · C crop · M markup · I details · Space play/pause animation · , . previous/next frame |
| Player | Space play/pause · Left/Right 5 s (Shift 30 s) · Up/Down volume · M mute · [ ] speed · = normal speed · G/H subtitle delay · Shift+G/H subtitle size · V subtitles on/off · F full screen |
| Text | Ctrl+S save · Ctrl+Shift+S save as · Ctrl+F find |
| Canvas | Ctrl+V paste a picture · Ctrl+C copy · Ctrl+S save · Ctrl+Z / Ctrl+Y undo/redo |

## Build from source

```powershell
powershell -ExecutionPolicy Bypass -File tools/get-ffmpeg.ps1   # once: the pinned LGPL FFmpeg build
cargo run                                                        # debug build
powershell -ExecutionPolicy Bypass -File tools/pack.ps1 -Install # release installer, installed on this PC
powershell -ExecutionPolicy Bypass -File tools/bench.ps1         # size, memory and CPU against the apps it replaces
```

Releases: see [RELEASING.md](RELEASING.md). Third-party software: [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
