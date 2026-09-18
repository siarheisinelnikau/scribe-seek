//! Updates come only from this repository's stable GitHub Releases. The downloaded
//! EXE is verified against GitHub's SHA-256 asset digest before a helper replaces
//! the running program. The helper is a temporary copy of this same executable.
use crate::{
    i18n::Locale,
    storage::{Storage, atomic_write},
};
use anyhow::{Context, Result, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const RELEASES_URL: &str = "https://github.com/siarheisinelnikau/scribe-seek/releases";
const LATEST_API: &str =
    "https://api.github.com/repos/siarheisinelnikau/scribe-seek/releases/latest";
pub const EXE_ASSET: &str = "scribe-seek-windows-x64.exe";
const MAX_EXE_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct Downloaded {
    pub release: Release,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    NoReleases,
    Available(Release),
    Downloading(f32),
    Ready(Downloaded),
    Failed(String),
}

enum UpdateEvent {
    Checked(Option<Release>, bool),
    Progress(f32),
    Downloaded(Downloaded),
    Failed(String),
}

pub struct Updater {
    pub state: UpdateState,
    events: Option<Receiver<UpdateEvent>>,
    cancel: Arc<AtomicBool>,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            state: UpdateState::Idle,
            events: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Updater {
    pub fn busy(&self) -> bool {
        self.events.is_some()
    }
    pub fn check(&mut self) {
        if self.busy() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.events = Some(receiver);
        self.state = UpdateState::Checking;
        std::thread::spawn(move || {
            let event = match check_latest(env!("CARGO_PKG_VERSION")) {
                Ok((release, exists)) => UpdateEvent::Checked(release, exists),
                Err(error) => UpdateEvent::Failed(format!("{error:#}")),
            };
            let _ = sender.send(event);
        });
    }
    pub fn download(&mut self, release: Release, storage: Storage) {
        if self.busy() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.events = Some(receiver);
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        self.state = UpdateState::Downloading(0.0);
        std::thread::spawn(move || {
            let result = download_release(release, &storage, &cancel, |progress| {
                let _ = sender.send(UpdateEvent::Progress(progress));
            });
            let event = match result {
                Ok(downloaded) => UpdateEvent::Downloaded(downloaded),
                Err(error) => UpdateEvent::Failed(format!("{error:#}")),
            };
            let _ = sender.send(event);
        });
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn poll(&mut self) {
        let events: Vec<_> = self
            .events
            .as_ref()
            .map_or_else(Vec::new, |r| r.try_iter().collect());
        for event in events {
            self.state = match event {
                UpdateEvent::Progress(p) => UpdateState::Downloading(p),
                UpdateEvent::Checked(release, exists) => {
                    self.events = None;
                    if let Some(release) = release {
                        UpdateState::Available(release)
                    } else if exists {
                        UpdateState::UpToDate
                    } else {
                        UpdateState::NoReleases
                    }
                }
                UpdateEvent::Downloaded(downloaded) => {
                    self.events = None;
                    UpdateState::Ready(downloaded)
                }
                UpdateEvent::Failed(error) => {
                    self.events = None;
                    UpdateState::Failed(error)
                }
            };
        }
    }
}
impl Drop for Updater {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout)))
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_per_read(Some(Duration::from_secs(10)))
        .build()
        .new_agent()
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<GithubAsset>,
}
#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

pub fn check_latest(current: &str) -> Result<(Option<Release>, bool)> {
    let mut response = match agent(25)
        .get(LATEST_API)
        .header(
            "User-Agent",
            concat!("ScribeSeek/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok((None, false)),
        Err(error) => return Err(error).context("Could not check GitHub for updates"),
    };
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 1_048_576,
        "GitHub release response is too large"
    );
    Ok((parse_release(&bytes, current)?, true))
}

fn parse_release(bytes: &[u8], current: &str) -> Result<Option<Release>> {
    let release: GithubRelease =
        serde_json::from_slice(bytes).context("Invalid GitHub release information")?;
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .context("Invalid release version")?;
    if release.draft
        || release.prerelease
        || !version.pre.is_empty()
        || version <= Version::parse(current)?
    {
        return Ok(None);
    }
    let asset = release
        .assets
        .into_iter()
        .find(|a| a.name == EXE_ASSET)
        .context("This release has no compatible Windows executable")?;
    let expected = format!("{RELEASES_URL}/download/{}/{EXE_ASSET}", release.tag_name);
    ensure!(
        asset.browser_download_url == expected,
        "Update download URL is not from this repository"
    );
    ensure!(
        (512..=MAX_EXE_SIZE).contains(&asset.size),
        "Invalid update file size"
    );
    let digest = asset
        .digest
        .and_then(|s| s.strip_prefix("sha256:").map(str::to_owned))
        .context("GitHub did not provide an update checksum")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid update checksum"
    );
    Ok(Some(Release {
        version: version.to_string(),
        url: expected,
        size: asset.size,
        sha256: digest.to_lowercase(),
    }))
}

fn unique_dir(storage: &Storage) -> Result<PathBuf> {
    let root = storage.root.join("updates");
    fs::create_dir_all(&root)?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let dir = root.join(format!("{}-{nonce}", std::process::id()));
    fs::create_dir(&dir)?;
    Ok(dir)
}

fn download_release(
    release: Release,
    storage: &Storage,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<Downloaded> {
    let directory = unique_dir(storage)?;
    let path = directory.join("download.exe");
    let result = (|| -> Result<()> {
        ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
        let mut response = agent(1800)
            .get(&release.url)
            .header(
                "User-Agent",
                concat!("ScribeSeek/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .context("Could not download the update")?;
        let mut input = response.body_mut().as_reader();
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mut buffer = vec![0u8; 128 * 1024];
        let mut received = 0;
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            received += count as u64;
            ensure!(received <= release.size, "Invalid update file size");
            output.write_all(&buffer[..count])?;
            progress(received as f32 / release.size as f32);
        }
        output.sync_all()?;
        drop(output);
        ensure!(received == release.size, "Update download was interrupted");
        verify_payload(&path, &release.sha256)?;
        ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    Ok(Downloaded { release, path })
}

fn verify_payload(path: &Path, expected: &str) -> Result<()> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    ensure!(
        (512..=MAX_EXE_SIZE).contains(&size),
        "Invalid update file size"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == expected,
        "Update checksum does not match"
    );
    ensure!(
        bytes.starts_with(b"MZ"),
        "Update is not a Windows x64 executable"
    );
    let offset = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
    let header = bytes
        .get(offset..offset.saturating_add(6))
        .context("Update is not a Windows x64 executable")?;
    ensure!(
        &header[..4] == b"PE\0\0" && header[4..6] == [0x64, 0x86],
        "Update is not a Windows x64 executable"
    );
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct UpdatePlan {
    target: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    sha256: String,
    parent_pid: u32,
    data_dir: PathBuf,
    source: Option<PathBuf>,
    locale: Locale,
}

pub fn prepare_restart(
    downloaded: &Downloaded,
    storage: &Storage,
    source: Option<PathBuf>,
    locale: Locale,
) -> Result<()> {
    #[cfg(not(windows))]
    anyhow::bail!("In-app updates are currently available on Windows only");
    #[cfg(windows)]
    {
        verify_payload(&downloaded.path, &downloaded.release.sha256)?;
        let target = fs::canonicalize(std::env::current_exe()?)?;
        let parent = target
            .parent()
            .context("Could not locate the application folder")?;
        let directory = unique_dir(storage)?;
        let token = directory.file_name().unwrap().to_string_lossy();
        let staged = parent.join(format!(".scribe-seek-{token}.pending.exe"));
        let backup = parent.join(format!(".scribe-seek-{token}.previous.exe"));
        let helper = directory.join("updater.exe");
        let result = (|| -> Result<()> {
            let mut input = fs::File::open(&downloaded.path)?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)
                .context(
                    "The application folder is not writable. Download the new version manually",
                )?;
            std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            drop(output);
            verify_payload(&staged, &downloaded.release.sha256)?;
            fs::copy(&target, &helper)?;
            let plan = UpdatePlan {
                target,
                staged: staged.clone(),
                backup,
                sha256: downloaded.release.sha256.clone(),
                parent_pid: std::process::id(),
                data_dir: storage.root.clone(),
                source,
                locale,
            };
            let plan_path = directory.join("plan.json");
            atomic_write(&plan_path, &serde_json::to_vec(&plan)?)?;
            use std::os::windows::process::CommandExt;
            Command::new(helper)
                .arg("--apply-update")
                .arg(plan_path)
                .creation_flags(0x08000000)
                .spawn()
                .context("Could not start the update helper")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(staged);
        }
        result
    }
}

pub fn helper_locale(path: &Path) -> Locale {
    read_plan(path).map_or(Locale::English, |plan| plan.locale)
}

fn read_plan(path: &Path) -> Result<UpdatePlan> {
    let file = fs::File::open(path)?;
    ensure!(file.metadata()?.len() <= 65_536, "Invalid update plan");
    Ok(serde_json::from_reader(file)?)
}

pub fn run_helper(path: &Path) -> Result<()> {
    let plan = read_plan(path)?;
    wait_for_parent(plan.parent_pid)?;
    replace_and_restart(&plan, |plan| {
        let mut command = Command::new(&plan.target);
        command.env("SCRIBE_SEEK_DATA_DIR", &plan.data_dir);
        if let Some(source) = &plan.source {
            command.arg(source);
        }
        command
            .spawn()
            .context("Could not restart the application")?;
        Ok(())
    })?;
    let _ = fs::remove_file(&plan.staged);
    let _ = fs::remove_file(path);
    Ok(())
}

fn replace_and_restart(
    plan: &UpdatePlan,
    launch: impl FnOnce(&UpdatePlan) -> Result<()>,
) -> Result<()> {
    ensure!(
        plan.target.is_absolute()
            && plan.staged.is_absolute()
            && plan.backup.is_absolute()
            && plan.target != plan.staged
            && plan.target != plan.backup
            && plan.staged != plan.backup,
        "Invalid update plan"
    );
    ensure!(
        plan.target.parent() == plan.staged.parent()
            && plan.target.parent() == plan.backup.parent(),
        "Invalid update plan"
    );
    ensure!(!plan.backup.exists(), "Update backup already exists");
    verify_payload(&plan.staged, &plan.sha256)?;
    fs::rename(&plan.target, &plan.backup).context("Could not replace the running application")?;
    if let Err(error) = fs::rename(&plan.staged, &plan.target) {
        fs::rename(&plan.backup, &plan.target).context("Could not restore the previous version")?;
        return Err(error).context("Could not install the update");
    }
    if let Err(error) = launch(plan) {
        // Move the rejected new version aside and restore the original name.
        fs::rename(&plan.target, &plan.staged).context("Could not restore the previous version")?;
        fs::rename(&plan.backup, &plan.target).context("Could not restore the previous version")?;
        return Err(error);
    }
    // Preserve the old executable as a rollback copy; never delete the user's original.
    Ok(())
}

#[cfg(windows)]
fn wait_for_parent(pid: u32) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    // SAFETY: a process handle is opened for synchronization only, used once,
    // and closed after the wait. The child never terminates the parent process.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                return Ok(());
            }
            return Err(error).context("Could not wait for the application to close");
        }
        let result = WaitForSingleObject(handle, 30_000);
        CloseHandle(handle);
        ensure!(
            result == WAIT_OBJECT_0,
            "The application did not close in time"
        );
    }
    Ok(())
}
#[cfg(not(windows))]
fn wait_for_parent(_: u32) -> Result<()> {
    anyhow::bail!("In-app updates are currently available on Windows only")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str) -> serde_json::Value {
        serde_json::json!({"tag_name": tag, "draft": false, "prerelease": false, "assets": [{
            "name": EXE_ASSET, "browser_download_url": format!("{RELEASES_URL}/download/{tag}/{EXE_ASSET}"),
            "size": 1024, "digest": format!("sha256:{}", "ab".repeat(32)) }]})
    }
    #[test]
    fn only_offers_newer_stable_versions_with_verified_repository_assets() {
        assert!(
            parse_release(&serde_json::to_vec(&release("v0.1.9")).unwrap(), "0.1.10")
                .unwrap()
                .is_none()
        );
        assert!(
            parse_release(&serde_json::to_vec(&release("v0.2.0")).unwrap(), "0.1.10")
                .unwrap()
                .is_some()
        );
        assert!(
            parse_release(
                &serde_json::to_vec(&release("v0.3.0-beta.1")).unwrap(),
                "0.1.0"
            )
            .unwrap()
            .is_none()
        );
        let mut bad = release("v0.2.0");
        bad["assets"][0]["browser_download_url"] = "https://example.com/file.exe".into();
        assert!(parse_release(&serde_json::to_vec(&bad).unwrap(), "0.1.0").is_err());
        let mut bad = release("v0.2.0");
        bad["assets"][0]["digest"] = serde_json::Value::Null;
        assert!(parse_release(&serde_json::to_vec(&bad).unwrap(), "0.1.0").is_err());
    }
    fn pe(marker: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
        bytes[128..134].copy_from_slice(b"PE\0\0\x64\x86");
        bytes[511] = marker;
        bytes
    }
    fn plan(dir: &Path) -> UpdatePlan {
        let bytes = pe(2);
        fs::write(dir.join("app.exe"), pe(1)).unwrap();
        fs::write(dir.join("new.exe"), &bytes).unwrap();
        UpdatePlan {
            target: dir.join("app.exe"),
            staged: dir.join("new.exe"),
            backup: dir.join("previous.exe"),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            parent_pid: 0,
            data_dir: dir.into(),
            source: None,
            locale: Locale::English,
        }
    }
    #[test]
    fn verifies_payload_and_restores_original_if_restart_fails() {
        let directory = tempfile::tempdir().unwrap();
        let plan = plan(directory.path());
        let result = replace_and_restart(&plan, |_| anyhow::bail!("simulated launch failure"));
        assert!(result.is_err());
        assert_eq!(fs::read(&plan.target).unwrap(), pe(1));
        assert!(!plan.backup.exists());
        fs::write(&plan.staged, pe(3)).unwrap();
        assert!(replace_and_restart(&plan, |_| Ok(())).is_err());
        assert_eq!(fs::read(&plan.target).unwrap(), pe(1));
    }
    #[test]
    fn installs_verified_payload_and_keeps_a_rollback_copy() {
        let directory = tempfile::tempdir().unwrap();
        let plan = plan(directory.path());
        replace_and_restart(&plan, |p| {
            assert_eq!(fs::read(&p.target)?, pe(2));
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(plan.backup).unwrap(), pe(1));
    }
}
