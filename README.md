# Rimelight Media

> **High-Performance Unified Media Application for Windows**  
> Built in Rust + Slint, packaged with Velopack, powered by trimmed LGPL FFmpeg. About 30 MB installed.

---

## 1. Overview

**Rimelight Media** is a compact, high-performance desktop application for pictures, video, music, text, screen snips, screen recording, webcam, and drawings. It replaces seven default Windows applications with a single lightweight tool:

| Replaces | With Rimelight Media Feature |
|---|---|
| **Photos** | Image Tab (viewing, comparisons, animated GIF/WebP, markup) |
| **Media Player / VLC** | Audio & Video Tabs (FFmpeg player, subtitles, waveform seek bar) |
| **Notepad** | Text Tab (editor, markdown split-preview, JSON validator) |
| **Snipping Tool** | Screen Snip (`Win+Shift+S`) & MP4 Screen Recording |
| **Camera** | Camera Page (webcam stills and video) |
| **Sound Recorder** | Voice Recorder (microphone capture to M4A/WAV/FLAC) |
| **Paint** | Canvas Page (drawing, shapes, transparency, image markup) |

---

## 2. Standard Documentation Suite

This repository adheres to the standardized 4-file documentation architecture:

| Document | Purpose |
|---|---|
| [**README.md**](file:///C:/Users/idant/RustRoverProjects/rimelight-media/README.md) | Simple overview of Rimelight Media with local development and build instructions (this file). |
| [**PROJECT.md**](file:///C:/Users/idant/RustRoverProjects/rimelight-media/PROJECT.md) | Comprehensive description: technical architecture, complete feature catalog, and shortcuts. |
| [**PLAN.md**](file:///C:/Users/idant/RustRoverProjects/rimelight-media/PLAN.md) | Progress tracker: live feature maturity matrix, pending verifications, and next steps. |
| [**AGENTS.md**](file:///C:/Users/idant/RustRoverProjects/rimelight-media/AGENTS.md) | Specific agent instructions: strict "build once" rule, Velopack release workflows, and local test rules. |

---

## 3. Local Development & Building from Source

### Prerequisites

- **Rust:** Latest stable toolchain (2024 edition).
- **PowerShell:** Windows 10/11 64-bit with script execution enabled.

### Build Steps

Run from the repository root:

```powershell
# 1. Download the pinned trimmed LGPL FFmpeg build (one-time setup)
powershell -ExecutionPolicy Bypass -File tools/get-ffmpeg.ps1

# 2. Run debug build
cargo run

# 3. Build & install local release package via Velopack
powershell -ExecutionPolicy Bypass -File tools/pack.ps1 -Install

# 4. Run resource benchmarks
powershell -ExecutionPolicy Bypass -File tools/bench.ps1
```

---

## 4. Keyboard Shortcuts Quick-Reference

- **Global:** `Print Screen` / `Win+Shift+S` (snip) · `Shift+Print Screen` (record) · `Ctrl+O` (open) · `Ctrl+N` (canvas) · `F11` (fullscreen).
- **Pictures:** `Left`/`Right` (nav) · `+`/`-`/`0`/`1` (zoom) · `R` (rotate) · `C` (crop) · `M` (markup) · `I` (details).
- **Player:** `Space` (play/pause) · `Left`/`Right` (seek) · `Up`/`Down` (volume) · `[` / `]` (speed) · `G`/`H` (subtitles).
- **Text:** `Ctrl+S` (save) · `Ctrl+Shift+S` (save as) · `Ctrl+F` (find).

---

## 5. License

MIT License. See [LICENSE.md](LICENSE.md). Third-party software notices: [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
