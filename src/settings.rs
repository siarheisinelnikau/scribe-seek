use crate::{
    i18n::Locale,
    storage::{Storage, atomic_write},
    theme::Theme,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ui_language: Locale,
    pub theme: Theme,
    pub speech_language: String,
    pub model: String,
    pub check_updates_on_startup: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui_language: Locale::English,
            theme: Theme::Light,
            speech_language: "auto".into(),
            model: "base".into(),
            check_updates_on_startup: true,
        }
    }
}

impl Settings {
    pub fn load(storage: &Storage) -> Result<Self> {
        let path = storage.root.join("settings.json");
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, storage: &Storage) -> Result<()> {
        atomic_write(
            &storage.root.join("settings.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn english_is_default_and_preferences_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::at(dir.path().to_owned()).unwrap();
        assert_eq!(
            Settings::load(&storage).unwrap().ui_language,
            Locale::English
        );
        let settings = Settings {
            ui_language: Locale::Italian,
            theme: Theme::Dark,
            speech_language: "fr".into(),
            model: "tiny".into(),
            ..Default::default()
        };
        settings.save(&storage).unwrap();
        assert_eq!(Settings::load(&storage).unwrap(), settings);
        std::fs::write(storage.root.join("settings.json"), b"broken").unwrap();
        assert!(Settings::load(&storage).is_err());
    }

    #[test]
    fn older_settings_keep_language_and_update_preferences() {
        let settings: Settings = serde_json::from_str(
            r#"{"ui_language":"ru","speech_language":"de","model":"small","check_updates_on_startup":false}"#,
        )
        .unwrap();
        assert_eq!(settings.ui_language, Locale::Russian);
        assert_eq!(settings.theme, Theme::Light);
        assert_eq!(settings.speech_language, "de");
        assert!(!settings.check_updates_on_startup);
    }
}
