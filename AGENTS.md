# Agent and maintainer notes

## Releases

### Publishing on GitHub, with auto-update

Use **Velopack + GitHub Releases** (the same installer rimelight-space uses), with the release built by GitHub Actions
when you push a version tag.

- **Installer:** `RimelightMedia-win-Setup.exe`. Installs per user (no admin, no UAC prompt) to
  `%LOCALAPPDATA%\RimelightMedia`, adds the Start menu shortcut and the Apps > Installed apps entry, and registers the
  file types (pictures, video, music) so you can pick Rimelight Media in Settings > Apps > Default apps. Uninstalling
  removes the file types and the start-with-Windows entry too.
- **Updates:** an installed copy checks the repository's latest release a few seconds after it opens, downloads it in
  the background (as a small delta when possible) and shows **Restart to update** in the top bar. If you just quit, the
  update is applied on the way out. Settings has **Check for updates**.
- **No server, no secrets:** the release workflow uses the built-in `GITHUB_TOKEN`. The repository must be **public**
  for installed copies to see its releases without a token (a private repo would need a token baked into the app, which
  isn't safe to ship).

### One-time setup

1. The repository is `Rimelight-Entertainment/rimelight-media`. `.github/workflows/release.yml` must be at its root.
2. In the repository settings > Actions > General, allow "Read and write permissions" for workflows.
3. Optional, recommended later: code signing (SmartScreen warns about unsigned installers for a while). Add a signing
   step with `vpk pack --signParams` once you have a certificate (Azure Trusted Signing is the cheapest route).

### Making a release

1. Bump `version` in `Cargo.toml` (for example `0.2.0`) and commit. The tag must equal this version.
2. `git tag v0.2.0` and `git push --tags`.
3. The **Release** workflow builds, packs and publishes "Rimelight Media v0.2.0" with the installer and update files.
   Installed copies pick it up on their next start.

### Building an installer on this PC

```powershell
powershell -ExecutionPolicy Bypass -File tools/pack.ps1 -Install
```

Builds the release, packs `releases/RimelightMedia-win-Setup.exe` and installs it silently. Then `tools/bench.ps1`
compares disk size, memory and CPU with VLC, Photos, Snipping Tool and Media Player. A local build has no GitHub
repository baked in, so it doesn't check for updates (pass `-Repo owner/name` to test that).

### Size

What ships: `rimelight-media.exe` (about 12 MB, the C runtime and the icon inside) plus five FFmpeg libraries, about
16 MB. FFmpeg is built in **rimelight-rs** (`tools/ffmpeg/build.sh`, its FFmpeg workflow) with only the formats the apps
play, trim and record (`tools/ffmpeg/options.txt`), plus dav1d for AV1. Its pin is `tools/ffmpeg/pin.txt`. To support
another format, add it to `options.txt` there, bump `REVISION` in `build.sh` and push: the workflow publishes
`ffmpeg-n9.0.2-rN` (the asset is still on this repository's releases until that workflow publishes one); then update
the tag and SHA-256 in `tools/get-ffmpeg.ps1` and in `pin.txt` together. `ffmpeg-sys-next` (the bindings this app
patches in) also comes from rimelight-rs (`vendor/ffmpeg-sys-next`, tag v0.2.0). The video engine (FFmpeg player, subtitles, speed, trimming, poster frames) is the `rimelight_media` crate and picture decoding, thumbnails, edits, markup and saving are the `rimelight_picture` crate, both in rimelight-rs (tag v0.2.0); this app keeps only its UI glue (`src/picture.rs` for Slint images, the clipboard and the icon). The test files used to check a new
build: every format in `samples/codecs` must decode
(`RIMELIGHT_MEDIA_SELFTEST=<folder> rimelight-media.exe`).

## Local checks

- `target/debug/rimelight-media.exe --screenshot <png> [--page image|audio|video|text|voice|camera] [file]` renders a
  page without showing a window. Set `APPDATA` to a temp folder so the user's running copy is untouched.
- Agents do not run cargo. After each agent batch, the main session runs `cargo build` and fixes what it reports. A
  batch isn't done until the build passes with no warnings, and the final report must say so.

## Next steps

- **Playlists and multiple files:** play and browse related files as one set: TV series (episodes in order, season
  folders), albums (track order, cover art), and image sets (a folder or a series of pictures, moved through in order).
- **Bulk file management:** replace tools like Mp3tag (tags and file names from one table, with a preview) and PowerToys
  Batch Rename (pattern-based renaming with a preview and undo).
- **Metadata viewer:** a Details panel for audio, video and text too (tags, codecs, encoding), not only for pictures.
- **Keyboard cheat sheet:** a Ctrl+? overlay listing the shortcuts for the current tab.

## Pending checks (verify on the owner's PC)

Built, not yet seen running. Open each and tick it off.

### Earlier UI fixes
- [ ] Tab switches tabs everywhere except in the Text editor, where it does nothing.
- [ ] Editor caret visible on JSON and code files.
- [ ] Recent list height matches a real row; Home matches the empty tabs.
- [ ] Picture tab empty state lines up with the other tabs (reported fixed).
- [ ] "Show in folder" sits right of Open on Image, Audio, Video and Text.
- [ ] Close buttons stay visible but disabled when nothing is open.
- [ ] Text toolbar has no stray divider after Discard.

### Details panels (Audio, Video, Text)
- [ ] Audio and Video: info button (or I) opens the panel; values look right for a song and a video.
- [ ] Text: info button opens the panel; the counts update after typing pauses.
- [ ] Text: decide whether the whole status bar hides in full screen (the panel already hides).

### Playlists (Audio and Video)
- [ ] Playlist button opens the left panel; the current file is highlighted; clicking a row plays it.
- [ ] Next and Previous buttons and the N and P keys move through the folder.
- [ ] A song or video moves on to the next file when it ends.
- [ ] Check the panel's list fills its height and lines up with the Details panel when both are open.

### Image slideshow
- [ ] Slideshow button (or S) starts and stops the slideshow; it advances every 4 seconds and wraps to the first picture.
- [ ] It stops when you edit a picture, enter crop, resize or markup, or leave the Image tab.

### Round 3 fixes (built, not yet seen running)
- [ ] Text editor: Tab does nothing while typing (it no longer switches tabs). Confirm the key reaches the editor on Windows.
- [ ] Text editor: the caret is the editor's own and sits right on JSON and code. Syntax colours are off for now.
- [ ] Playlist panel and button now sit to the left of Details.
- [ ] Several audio or video files opened together make a playlist of exactly those files, and the panel lists them all (scrolls when long).
- [ ] Continue button sits to the right of Repeat; turning it off stops auto-advance.

## Open questions (answer when you check)

- [ ] Button order on Audio and Video: Prev, Next, Play (current) or Prev, Play, Next?
- [ ] Should playing from the playlist add each file to Recent, or only files you open directly?
- [ ] Should auto-advance (audio and video) be a setting, or always on?
- [ ] Which screen shows the picture tab shift, if it still appears? (Toolbar, empty state or viewer.)
- [ ] Should the text Details button hide in full screen too?
- [ ] Should the slideshow interval (4 seconds) be a setting?
