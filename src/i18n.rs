use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Locale {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "ru")]
    Russian,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "it")]
    Italian,
}

pub const LOCALES: [Locale; 6] = [
    Locale::English,
    Locale::Russian,
    Locale::French,
    Locale::German,
    Locale::Spanish,
    Locale::Italian,
];

type Catalog = BTreeMap<String, String>;

fn catalogs() -> &'static [Catalog; 6] {
    static CATALOGS: OnceLock<[Catalog; 6]> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        [
            include_str!("../locales/en.json"),
            include_str!("../locales/ru.json"),
            include_str!("../locales/fr.json"),
            include_str!("../locales/de.json"),
            include_str!("../locales/es.json"),
            include_str!("../locales/it.json"),
        ]
        .map(|source| serde_json::from_str(source).expect("validated bundled translation catalog"))
    })
}

impl Locale {
    pub fn code(self) -> &'static str {
        ["en", "ru", "fr", "de", "es", "it"][self as usize]
    }
    pub fn native_name(self) -> &'static str {
        self.tr("Native language name")
    }
    pub fn tr(self, key: &'static str) -> &'static str {
        catalogs()[self as usize]
            .get(key)
            .map(String::as_str)
            .unwrap_or(key)
    }
    pub fn format(self, key: &'static str, values: &[(&str, String)]) -> String {
        let mut text = self.tr(key).to_owned();
        for (name, value) in values {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    pub fn segments(self, count: usize) -> String {
        let key = match self {
            Self::Russian if count % 10 == 1 && count % 100 != 11 => "{count} segment",
            Self::Russian
                if (2..=4).contains(&(count % 10)) && !(12..=14).contains(&(count % 100)) =>
            {
                "{count} segments (few)"
            }
            Self::French if count <= 1 => "{count} segment",
            Self::Russian => "{count} segments",
            _ if count == 1 => "{count} segment",
            _ => "{count} segments",
        };
        self.format(key, &[("count", count.to_string())])
    }

    /// Translate application error contexts while retaining native technical details.
    /// Messages stay in English internally, so changing the UI language also updates
    /// notices and running task labels that were created before the change.
    pub fn message(self, text: &str) -> String {
        if let Some(value) = catalogs()[self as usize].get(text) {
            return value.clone();
        }
        for (prefix, key, name) in [
            (
                "Could not read saved transcripts: ",
                "Could not read saved transcripts: {count}",
                "count",
            ),
            (
                "Could not read the library: ",
                "Could not read the library: {error}",
                "error",
            ),
            (
                "Could not load settings: ",
                "Could not load settings: {error}",
                "error",
            ),
            (
                "Could not save settings: ",
                "Could not save settings: {error}",
                "error",
            ),
            ("Saved: ", "Saved: {path}", "path"),
            ("Could not save ", "Could not save {path}", "path"),
            ("Could not seek: ", "Could not seek: {error}", "error"),
        ] {
            if let Some(value) = text.strip_prefix(prefix) {
                return self.format(key, &[(name, self.message(value))]);
            }
        }
        text.split(": ")
            .map(|part| {
                catalogs()[self as usize]
                    .get(part)
                    .map_or(part, String::as_str)
            })
            .collect::<Vec<_>>()
            .join(": ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn placeholders(text: &str) -> Vec<&str> {
        let mut names = text
            .split('{')
            .skip(1)
            .filter_map(|s| s.split_once('}').map(|p| p.0))
            .collect::<Vec<_>>();
        names.sort();
        names
    }
    #[test]
    fn translations_are_complete_and_keep_format_arguments() {
        let reference = &catalogs()[0];
        for catalog in catalogs() {
            assert_eq!(
                reference.keys().collect::<Vec<_>>(),
                catalog.keys().collect::<Vec<_>>()
            );
            for (key, value) in catalog {
                assert!(!value.trim().is_empty(), "empty translation: {key}");
                assert_eq!(
                    placeholders(key),
                    placeholders(value),
                    "placeholder mismatch: {key}"
                );
            }
        }
    }
    #[test]
    fn notices_and_error_contexts_follow_the_current_locale() {
        assert_eq!(
            Locale::French.message("Operation cancelled"),
            "Opération annulée"
        );
        let error = Locale::German
            .message("Could not read the library: Source file is unavailable: OS error 2");
        assert!(error.contains("Quelldatei"));
        assert!(error.ends_with("OS error 2"));
        assert_eq!(
            Locale::English.message("Saved: C:\\audio\\lecture.txt"),
            "Saved: C:\\audio\\lecture.txt"
        );
    }
}
