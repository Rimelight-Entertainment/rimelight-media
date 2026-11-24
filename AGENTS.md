# Agent and maintainer notes

## How we work: build ONCE, at the very end (owner rule, overrides everything else)

Builds fill the owner's disk, lag the whole PC and used to take most of every session. So:

- **Write the features first, in big batches.** Many steps, across repos if needed, with no build, check, test,
  clippy, gate, measurement or screenshot in between.
- **Then run ONE build and test pass** to find what broke along the way, fix it all, and commit.
- **Never** build per step, per agent, per repo, before a push, after a pin bump, to "measure first", or to "just check".
  A plan document's per-step gate (`gate quick`, "gate per run", "measure first") does not override this.
- Subagents never run cargo, `vp`, npm builds or tests at all.
- Before the one build, if the disk has under 40 GB free, delete `debug/incremental` in the target folder.
- Never sit idle waiting on a build. If something must be built, it is the final pass.

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
- **Files tab (new, after Home):** a tab with a yellow folder icon for a simple file explorer. Scope, decided:
  - Browse and open (double-click opens a file in its own tab; folders open in the tab), copy, cut and paste, rename,
    new folder, and delete to the Recycle Bin (never permanent).
  - Batch rename: pattern-based (`{n}` counter with start and padding, `{name}`, `{ext}`), optional find and replace,
    with a preview and undo. Undo covers the batch rename, rename, new folder and move; copies and Recycle Bin deletes
    are not undone.
  - Archives: open `.zip`, `.7z` and `.rar` (password when needed; `.rar` is read-only, through the `unrar` crate, which
    is MIT/Apache for the Rust side and bundles unrar's C++ source). `.7z` through `sevenz-rust2`, pure Rust. Create
    `.zip` only, with AES-256 and an optional password (no ZipCrypto). Archives show their entries in the same table,
    with extract-here and extract-to.
  - Archive files get their own icons in the file explorer, like the other file types.
  - Shared pieces stay in rimelight-media. Nothing is added to `rimelight_ui` for this tab.
- **Text file fixes:** `.ts` files are currently shown as video and must show as text. `.jsx` and `.tsx` must be added to
  the supported text file list.

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

### Files tab (built, not yet seen running)
- [ ] Files nav button (yellow folder) sits after Home; the tab opens in the home folder.
- [ ] Double-click opens a folder; a picture, video, audio or text file opens in its own tab; Up and the trail work.
- [ ] Click, Ctrl-click and Shift-click select; the status bar counts them; a column header click sorts (folders stay first).
- [ ] New folder, rename (single selection), copy, cut and paste (into this folder; a folder can't go into itself).
- [ ] Keys on the Files tab: F2 renames the one selected item, Delete asks then recycles, Ctrl+C, Ctrl+X, Ctrl+V and Ctrl+Z work, and none fire while a dialog is open. Tab goes Home, Files, Image.
- [ ] Delete asks first and the items land in the Recycle Bin.
- [ ] Batch rename: the preview updates as you type; a problem row blocks Rename; the names change; Undo brings them back.
- [ ] Undo button: reverses the last rename, move or new folder (copies and Recycle Bin deletes are not undone).
- [ ] New archive: a zip of the selection; with a password it opens only with that password (7-Zip or this app; Explorer can't open AES zips).
- [ ] Open a zip, 7z and rar: a protected one asks for the password, and a wrong one asks again.
- [ ] Extract and Extract to: the files land in the new folder; a protected archive asks for its password.
- [ ] A file opened from inside an archive is extracted to %TEMP%\RimelightMedia\archive first (the whole archive).
- [ ] Every row has a file-type tile as Explorer does: yellow folder, grey drive, orange archive, lock (Encrypted), and the app's picture, video, audio and text tiles; any other type gets a neutral file tile.
- [ ] Toolbar is the same as the other tabs (same colour, height and hairline) and sits at the top; the trail is in its own band under it.
- [ ] Every Files button shows a tooltip on hover. The media app had no tooltip overlay before; this batch adds it for every tab.
- [ ] New folder uses the folder-plus icon (added to media's icons from Lucide, the set the app uses).
- [ ] Sidebar sections: Drives (each drive with its folders, Proton Drive or other cloud folders, Recycle Bin), Quick access (Desktop, Documents, Downloads, Pictures, Music, Videos), Pinned. Recycle Bin opens Windows' own Recycle Bin window.
- [ ] Pin button pins the one selected folder; its x removes the pin; pins survive a restart (pinned.txt in %APPDATA%\Rimelight Media).
- [ ] Shift click selects the folders from the last plain or Ctrl click down to the one clicked, in sidebar order.
- [ ] External drives appear as their drive letters. There is no removable-drive check yet (it needs a platform call), and Google Drive shows as its drive letter (G:) when it mounts one, or as a "Google Drive" folder.
- [ ] Ctrl and Shift click folders in the tree to multi-select them; Copy, Cut, Paste, Delete, Rename (one folder), Batch rename and New archive then act on those folders. A list click clears the tree selection.
- [ ] Folder tree on the left: drives at the top, arrows open and close folders, the folder you are in is highlighted and its parents are open. Clicking a row opens it.
- [ ] Details panel on the right: for one selection it shows type, location, size, items (folders), modified and created; with nothing selected it shows the folder you are in; with several it shows the count and total size.
- [ ] Double-click opens a folder (the list no longer rebuilds on every click, which used to lose the second click).
- [ ] Up and Down move the selection and scroll the list; Enter opens the selected item; Backspace goes up.

## Open questions (answer when you check)

- [ ] Button order on Audio and Video: Prev, Next, Play (current) or Prev, Play, Next?
- [ ] Should playing from the playlist add each file to Recent, or only files you open directly?
- [ ] Should auto-advance (audio and video) be a setting, or always on?
- [ ] Which screen shows the picture tab shift, if it still appears? (Toolbar, empty state or viewer.)
- [ ] Should the text Details button hide in full screen too?
- [ ] Should the slideshow interval (4 seconds) be a setting?
