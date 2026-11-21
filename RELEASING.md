# Publishing Rimelight Media on GitHub, with auto-update

## Recommendation

Use **Velopack + GitHub Releases** (the same installer rimelight-space uses, decision 0047), with the release built by
GitHub Actions when you push a version tag.

- **Installer:** `RimelightMedia-win-Setup.exe`. Installs per user (no admin, no UAC prompt) to
  `%LOCALAPPDATA%\RimelightMedia`, adds the Start menu shortcut and the Apps > Installed apps entry, and registers the
  file types (pictures, video, music) so you can pick Rimelight Media in Settings > Apps > Default apps. Uninstalling
  removes the file types and the start-with-Windows entry too.
- **Updates:** an installed copy checks the repository's latest release a few seconds after it opens, downloads it in the
  background (as a small delta when possible) and shows **Restart to update** in the top bar. If you just quit, the update
  is applied on the way out. Settings has **Check for updates**.
- **No server, no secrets:** the release workflow uses the built-in `GITHUB_TOKEN`. The repository must be **public** for
  installed copies to see its releases without a token (a private repo would need a token baked into the app, which isn't
  safe to ship).

## One-time setup

1. Create the repository on GitHub and push the contents of `app/` as its root (`.github/workflows/release.yml` must end up
   at the repository root).
2. In the repository settings > Actions > General, allow "Read and write permissions" for workflows.
3. Optional, recommended later: code signing (SmartScreen warns about unsigned installers for a while). Add a signing step
   with `vpk pack --signParams` once you have a certificate (Azure Trusted Signing is the cheapest route).

## Making a release

1. Bump `version` in `Cargo.toml` (for example `0.2.0`) and commit.
2. `git tag v0.2.0` and `git push --tags`.
3. The **Release** workflow builds, packs and publishes "Rimelight Media v0.2.0" with the installer and update files.
   Installed copies pick it up on their next start.

## Building an installer on this PC

```powershell
powershell -ExecutionPolicy Bypass -File tools/pack.ps1 -Install
```

Builds the release, packs `releases/RimelightMedia-win-Setup.exe` and installs it silently. Then
`tools/bench.ps1` compares disk size, memory and CPU with VLC, Photos, Snipping Tool and Media Player. A local build has
no GitHub repository baked in, so it doesn't check for updates (pass `-Repo owner/name` to test that).

## Size

What ships: `rimelight-media.exe` (about 12 MB, the C runtime and the icon inside) plus five FFmpeg libraries, about 16 MB:
FFmpeg is built by the **FFmpeg** workflow (`tools/ffmpeg/build.sh`) with only the formats the app plays, trims and
records (`tools/ffmpeg/options.txt`), plus dav1d for AV1. To support another format, add it to `options.txt`, bump
`REVISION` in `build.sh` and push: the workflow publishes `ffmpeg-n9.0.2-rN`; then update the tag and SHA-256 in
`tools/get-ffmpeg.ps1`. The test files used to check a new build: every format in `shots/codecs` must decode
(`RIMELIGHT_MEDIA_SELFTEST=<folder> rimelight-media.exe`).
