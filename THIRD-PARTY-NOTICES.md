# Third-party software in Rimelight Media

- **FFmpeg** (avcodec, avformat, avutil, swresample, swscale), LGPL 2.1 or later, shipped as unmodified DLLs from the
  BtbN FFmpeg-Builds release `autobuild-2026-11-19-12-39` (`ffmpeg-n9.0.2-58-g8c7ef49002-win64-lgpl-shared-9.0`).
  Source: https://github.com/FFmpeg/FFmpeg (commit 8c7ef49002, release/9.0 branch) and the build scripts at
  https://github.com/BtbN/FFmpeg-Builds. Licence text: `licenses/FFmpeg-LICENSE.txt`. You may replace the DLLs with your
  own build of the same FFmpeg major version.
- **Slint** 1.18.1, under the Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0 (the About section in
  Settings carries the required notice).
- **Lucide icons**, ISC licence (`licenses/Lucide-LICENSE.txt`).
- **Velopack** (installer and updates), MIT licence.
- Rust crates: image, resvg, cpal, global-hotkey, arboard, rfd, winreg, windows-sys and their dependencies, under MIT
  and/or Apache-2.0 (see each crate on crates.io).
