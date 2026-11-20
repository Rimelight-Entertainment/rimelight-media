//! Installer hooks and self-update with Velopack (the rimelight-space installer, decision 0047), fed from GitHub
//! Releases. An installed copy checks the repository's latest release in the background after it opens, downloads it,
//! and offers "Restart to update"; when the person just quits, the update is applied on the way out, so the next start
//! is already new. A copy that isn't installed (a build from source) never checks.

use velopack::sources::{AutoSource, FileSource, GithubSource};
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, VelopackApp};

/// "owner/name" of the GitHub repository whose releases feed updates. The release workflow sets it from
/// GITHUB_REPOSITORY when it builds; a local build without it doesn't check for updates.
const REPO: Option<&str> = option_env!("RIMELIGHT_MEDIA_REPO");

/// Runs the Velopack install, update and uninstall hooks. Must be the first call in `main`: during a hook it does its
/// work and exits the process.
pub fn init() {
    VelopackApp::build()
        .on_after_install_fast_callback(|_| {
            let _ = crate::install::register_file_types();
        })
        .on_after_update_fast_callback(|_| {
            let _ = crate::install::register_file_types();
        })
        .on_before_uninstall_fast_callback(|_| crate::install::unregister_all())
        .run();
}

/// The feed: RIMELIGHT_MEDIA_UPDATE_URL (a folder or web address, for testing a release locally) or the GitHub
/// repository's releases.
fn manager() -> Option<UpdateManager> {
    if let Ok(url) = std::env::var("RIMELIGHT_MEDIA_UPDATE_URL") {
        return UpdateManager::new(AutoSource::new(&url), None, None).ok();
    }
    let repo = REPO?;
    UpdateManager::new(GithubSource::new(&format!("https://github.com/{repo}"), None, false), None, None).ok()
}

/// The installed version, or None when this copy isn't installed.
pub fn installed_version() -> Option<String> {
    UpdateManager::new(FileSource::new("."), None, None).ok().map(|m| m.get_current_version_as_string())
}

pub fn feed_name() -> String {
    match (std::env::var("RIMELIGHT_MEDIA_UPDATE_URL"), REPO) {
        (Ok(url), _) => url,
        (_, Some(repo)) => format!("github.com/{repo}/releases"),
        _ => String::new(),
    }
}

pub fn releases_page() -> Option<String> {
    REPO.map(|r| format!("https://github.com/{r}/releases"))
}

pub enum Outcome {
    /// Not installed, or built without a feed.
    Off,
    UpToDate,
    /// Downloaded and waiting for a restart.
    Ready(Box<UpdateInfo>),
    Failed(String),
}

/// Checks and downloads (blocking; call from a worker thread).
pub fn check_and_download() -> Outcome {
    let Some(m) = manager() else { return Outcome::Off };
    match m.check_for_updates() {
        Ok(UpdateCheck::UpdateAvailable(info)) => match m.download_updates(&info, None) {
            Ok(()) => Outcome::Ready(info),
            Err(e) => Outcome::Failed(format!("Could not download the update: {e}")),
        },
        Ok(_) => Outcome::UpToDate,
        Err(e) => Outcome::Failed(format!("Could not check for updates: {e}")),
    }
}

/// Applies a downloaded update and restarts into it.
pub fn apply_now(info: &UpdateInfo) -> Result<(), String> {
    let m = manager().ok_or("not installed")?;
    m.apply_updates_and_restart(&info.TargetFullRelease).map_err(|e| e.to_string())
}

/// Applies a downloaded update after this process exits, without restarting.
pub fn apply_on_exit(info: &UpdateInfo) {
    if let Some(m) = manager() {
        let _ = m.wait_exit_then_apply_updates(&info.TargetFullRelease, true, false, Vec::<String>::new());
    }
}
