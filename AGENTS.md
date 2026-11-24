# Agent & Maintainer Instructions — Rimelight Media

> **Role & Mission:** Expert Rust & Slint developer maintaining the Rimelight Media application.  
> **Core Documentation Suite:**
> - [**README.md**](file:///C:/Users/idant/RustRoverProjects/rimelight-media/README.md): Simple overview of Rimelight Media with local development and build instructions.
> - [**PROJECT.md**](file:///C:/Users/idant/RustRoverProjects/rimelight-media/PROJECT.md): Comprehensive system specification, architecture, feature catalog, and shortcuts.
> - [**PLAN.md**](file:///C:/Users/idant/RustRoverProjects/rimelight-media/PLAN.md): Live feature maturity matrix, pending verifications, and next engineering steps.

---

## How we work: build ONCE, at the very end (owner rule, overrides everything else)

Builds fill the owner's disk, lag the whole PC and used to take most of every session. So:

- **Write the features first, in big batches.** Many steps, across repos if needed, with no build, check, test, clippy, gate, measurement or screenshot in between.
- **Then run ONE build and test pass** to find what broke along the way, fix it all, and commit.
- **Never** build per step, per agent, per repo, before a push, after a pin bump, to "measure first", or to "just check".
- **Subagents never run cargo, `vp`, npm builds or tests at all.**
- Before the one build, if the disk has under 40 GB free, delete `debug/incremental` in the target folder.
- Never sit idle waiting on a build. If something must be built, it is the final pass.

---

## Releases & Distribution

### Publishing on GitHub with Auto-Update

Use **Velopack + GitHub Releases** with releases built by GitHub Actions when a version tag is pushed:

- **Installer:** `RimelightMedia-win-Setup.exe`. Installs per user (no admin, no UAC prompt) to `%LOCALAPPDATA%\RimelightMedia`, adds Start menu shortcut, registers file types (pictures, video, music), and registers with Default Apps.
- **Updates:** Installed copies check the repository's latest release a few seconds after startup, download delta packages in the background, and prompt to restart.
- **Making a Release:**
  1. Bump `version` in `Cargo.toml`.
  2. Commit and push to main.
  3. Create and push tag `vX.Y.Z`:
     ```powershell
     git tag v0.X.X
     git push origin v0.X.X
     ```
  4. GitHub Actions builds the release and attaches `RimelightMedia-win-Setup.exe` and `.nupkg` assets to the release.

### Local Packaging

To test the installer locally:
```powershell
powershell -ExecutionPolicy Bypass -File tools/pack.ps1 -Install
```

---

## Local Verification Guidelines

- **Workspace Boundaries:** Never run cargo builds for individual small edits. Batch UI and logic updates.
- **FFmpeg DLLs:** Ensure `tools/get-ffmpeg.ps1` has been executed once so DLLs are present in `<target folder>/ffmpeg/current`.
- **UI Testing:** Test visual changes in both light and dark mode, and verify that layout scales gracefully across 100% and 150% Windows display scaling.
