//! Product adapter for the shared release, download, verification and installation engine.
use crate::{i18n::Locale, storage::Storage};
use anyhow::Result;
use rust_auto_updator::{Config, Event, Prepared, Service};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub const RELEASES_URL: &str = "https://github.com/siarheisinelnikau/scribe-seek/releases";
pub const LEGACY_EXE_ASSET: &str = "scribe-seek-windows-x64.exe";

pub fn config() -> Result<Config> {
    let mut config = Config::new(
        "siarheisinelnikau",
        "scribe-seek",
        env!("CARGO_PKG_VERSION"),
        "ScribeSeek",
    )
    .map_err(anyhow::Error::msg)?;
    config.max_executable_size = 256 * 1024 * 1024;
    config.expected_subsystem = Some(2);
    config.allow_asset_digest = true;
    config.legacy_executable_names = vec![LEGACY_EXE_ASSET.into()];
    config.install.stage_prefix = ".scribe-seek-update-".into();
    config.install.startup_health_timeout = Some(std::time::Duration::from_secs(30));
    Ok(config)
}
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub size: u64,
    inner: Box<rust_auto_updator::Release>,
}
impl From<rust_auto_updator::Release> for Release {
    fn from(release: rust_auto_updator::Release) -> Self {
        Self {
            version: release.version.to_string(),
            size: release.size(),
            inner: Box::new(release),
        }
    }
}
#[derive(Clone)]
pub struct Downloaded {
    pub release: Release,
    prepared: Arc<Mutex<Option<Prepared>>>,
}
impl std::fmt::Debug for Downloaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Downloaded")
            .field("release", &self.release)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(f32),
    Ready(Downloaded),
    Failed(String),
}

pub struct Updater {
    pub state: UpdateState,
    service: Option<Service>,
    context: egui::Context,
    downloading: Option<Release>,
}
impl Updater {
    pub fn new(context: egui::Context) -> Self {
        Self {
            state: UpdateState::Idle,
            service: None,
            context,
            downloading: None,
        }
    }
    pub fn busy(&self) -> bool {
        matches!(
            self.state,
            UpdateState::Checking | UpdateState::Downloading(_)
        )
    }
    fn start(&mut self, config: Config) -> Result<()> {
        let context = self.context.clone();
        self.service = Some(
            Service::new(config, move || context.request_repaint()).map_err(anyhow::Error::msg)?,
        );
        Ok(())
    }
    pub fn check(&mut self) {
        if self.busy() {
            return;
        }
        let result = (|| {
            self.start(config()?)?;
            self.service
                .as_ref()
                .unwrap()
                .check()
                .map_err(anyhow::Error::msg)
        })();
        self.state = match result {
            Ok(()) => UpdateState::Checking,
            Err(error) => UpdateState::Failed(error.to_string()),
        };
    }
    pub fn download(&mut self, release: Release, storage: Storage, source: Option<PathBuf>) {
        if self.busy() {
            return;
        }
        let result = (|| -> Result<()> {
            let mut config = config()?;
            config.install.restart_args = restart_args(&storage, source);
            config.install.rollback_args = config.install.restart_args.clone();
            self.start(config)?;
            self.service
                .as_ref()
                .unwrap()
                .download(release.inner.clone())
                .map_err(anyhow::Error::msg)
        })();
        match result {
            Ok(()) => {
                self.downloading = Some(release);
                self.state = UpdateState::Downloading(0.0);
            }
            Err(error) => self.state = UpdateState::Failed(error.to_string()),
        }
    }
    pub fn cancel(&self) {
        if let Some(service) = &self.service {
            service.cancel_download();
        }
    }
    pub fn poll(&mut self) {
        let events: Vec<_> = self
            .service
            .as_ref()
            .map_or_else(Vec::new, |service| service.events.try_iter().collect());
        for event in events {
            self.state = match event {
                Event::Checked(Ok(Some(release))) => {
                    UpdateState::Available(Release::from(*release))
                }
                Event::Checked(Ok(None)) => UpdateState::UpToDate,
                Event::Progress { received, total } => UpdateState::Downloading(if total == 0 {
                    0.0
                } else {
                    received as f32 / total as f32
                }),
                Event::Ready(Ok(prepared)) => UpdateState::Ready(Downloaded {
                    release: self.downloading.take().expect("download release"),
                    prepared: Arc::new(Mutex::new(Some(prepared))),
                }),
                Event::Checked(Err(error)) | Event::Ready(Err(error)) => UpdateState::Failed(error),
            };
        }
    }
}
fn restart_args(storage: &Storage, source: Option<PathBuf>) -> Vec<String> {
    let mut args = vec![
        "--data-dir".into(),
        storage.root.to_string_lossy().into_owned(),
    ];
    if let Some(source) = source {
        args.push("--source".into());
        args.push(source.to_string_lossy().into_owned());
    }
    args
}
pub fn prepare_restart(
    downloaded: &Downloaded,
    storage: &Storage,
    source: Option<PathBuf>,
    _locale: Locale,
) -> Result<()> {
    let mut guard = downloaded.prepared.lock().expect("prepared update");
    let prepared = guard
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("The update has already been handed off."))?;
    prepared
        .set_restart_args(restart_args(storage, source))
        .map_err(anyhow::Error::msg)?;
    rust_auto_updator::handoff(prepared).map_err(anyhow::Error::msg)
}
pub fn check_latest(current: &str) -> Result<(Option<Release>, bool)> {
    let mut config = config()?;
    config.current_version = current.parse()?;
    rust_auto_updator::check_latest(&config)
        .map(|release| (release.map(Release::from), true))
        .map_err(anyhow::Error::msg)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_preserves_explicit_storage_and_source() {
        let storage = Storage {
            root: PathBuf::from("C:/Data with spaces"),
        };
        assert_eq!(
            restart_args(&storage, Some(PathBuf::from("C:/Audio file.wav"))),
            [
                "--data-dir",
                "C:/Data with spaces",
                "--source",
                "C:/Audio file.wav"
            ]
        );
    }
}
