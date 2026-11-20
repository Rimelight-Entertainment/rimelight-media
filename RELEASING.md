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

What ships: `rimelight-media.exe` plus five FFmpeg libraries (avcodec, avformat, avutil, swresample, swscale). The
FFmpeg device and filter libraries are not used (screen grabbing uses Windows' own calls; playback speed is done in
the app), which saves 37 MB. avcodec (90 MB) is most of what's left: the prebuilt FFmpeg includes every encoder and
decoder. A custom FFmpeg build with only the codecs needed would bring the whole app to roughly 20-30 MB; that needs an
MSYS2 toolchain and is the next size step.
