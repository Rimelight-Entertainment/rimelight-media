# Third-party software in Rimelight Media

- **FFmpeg** n9.0.2 (avcodec, avformat, avutil, swresample, swscale), LGPL 2.1 or later, unmodified source built with
  only the formats this app uses by `tools/ffmpeg/build.sh` (options in `tools/ffmpeg/options.txt`, also in
  `licenses/FFmpeg-BUILD-INFO.txt`). Source: https://github.com/FFmpeg/FFmpeg/tree/n9.0.2. Licence text:
  `licenses/FFmpeg-LICENSE.txt`. You may replace the DLLs with your own build of the same FFmpeg major version.
- **dav1d** 1.5.5 (AV1 decoding, inside avcodec), BSD 2-clause (`licenses/dav1d-LICENSE.txt`).
- **Slint** 1.18.1, under the Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0 (the About section in
  Settings carries the required notice).
- **Lucide icons**, ISC licence (`licenses/Lucide-LICENSE.txt`).
- **Velopack** (installer and updates), MIT licence.
- **rawler** 0.7 (camera RAW decoding), LGPL-2.1. Source: https://crates.io/crates/rawler. Unmodified, used as a Rust
  library; you may replace it with another version of the same crate under the LGPL terms.
- Rust crates: image, resvg, cpal, global-hotkey, arboard, rfd, winreg, windows-sys and their dependencies, under MIT
  and/or Apache-2.0 (see each crate on crates.io).
