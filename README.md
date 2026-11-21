# rimelight-media

One small Windows app for pictures, video, music, screenshots and screen recording: a replacement for the Photos app,
Media Player, VLC and the Snipping Tool. Rust + Slint, FFmpeg for video and sound.

## Install

Download `RimelightMedia-win-Setup.exe` from the [latest release](https://github.com/Rimelight-Entertainment/rimelight-media/releases/latest)
and run it. It installs for your user only (no admin), adds a Start menu entry, and updates itself when a new release
comes out. Then open **Settings > Install, updates and default apps > Open Windows default apps** to make it the default
for pictures, video and music. Uninstall from Windows Settings > Apps > Installed apps.

## What it does

- **Pictures:** PNG, JPEG, WebP, SVG, GIF, BMP, ICO, HEIC/HEIF, AVIF and camera RAW (view only; save as PNG or JPEG). Zoom and pan, browse the folder, animated GIF/WebP with play, pause
  and frame-by-frame stepping. Crop, rotate, flip, resize, markup (pen, highlighter, arrow, box, text), save and copy.
- **Video and music:** MP4, MKV, WebM, MOV, AVI, MPEG and more; MP3, FLAC, WAV, OGG, Opus, M4A and more. Audio track
  choice, subtitles (embedded text and picture subtitles, or SRT/VTT/ASS files) with delay and size, playback speed
  0.25x to 3x, repeat, frame snapshots, fast trimming.
- **Capture:** Print Screen snips an area, a window or the whole screen (copied, saved and opened for markup). Screen
  recording to MP4 with the computer's sound and/or microphone. Voice recorder.

## Keys

| Where | Keys |
|---|---|
| Anywhere | Print Screen snip · Alt+Print Screen window · Ctrl+Print Screen whole screen · Shift+Print Screen record · Ctrl+O open · F11 full screen |
| Pictures | Left/Right previous/next · + / - / 0 / 1 zoom · R rotate · C crop · M markup · I details · Space play/pause animation · , . previous/next frame |
| Player | Space play/pause · Left/Right 5 s (Shift 30 s) · Up/Down volume · M mute · [ ] speed · = normal speed · G/H subtitle delay · Shift+G/H subtitle size · V subtitles on/off · F full screen |

## Build from source

```powershell
powershell -ExecutionPolicy Bypass -File tools/get-ffmpeg.ps1   # once: the pinned LGPL FFmpeg build
cargo run                                                        # debug build
powershell -ExecutionPolicy Bypass -File tools/pack.ps1 -Install # release installer, installed on this PC
powershell -ExecutionPolicy Bypass -File tools/bench.ps1         # size, memory and CPU against the apps it replaces
```

Releases: see [RELEASING.md](RELEASING.md). Third-party software: [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
