use crate::i18n::Locale;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Debug)]
pub struct SpeechLanguage {
    pub code: &'static str,
    pub model_supported: bool,
}

pub fn speech_languages() -> Vec<SpeechLanguage> {
    (0..=whisper_rs::get_lang_max_id())
        .filter_map(|id| {
            let code = whisper_rs::get_lang_str(id)?;
            // Explicit product preference: do not offer these in either language selector.
            if matches!(code, "uk" | "be") {
                return None;
            }
            // All currently bundled model choices use the pre-v3, 99-language vocabulary.
            Some(SpeechLanguage {
                code,
                model_supported: id < 99,
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct Names {
    languages: BTreeMap<String, BTreeMap<String, String>>,
}

pub fn language_name(locale: Locale, code: &str) -> &str {
    if code == "auto" {
        return locale.tr("Detect automatically");
    }
    static NAMES: OnceLock<Names> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        serde_json::from_str(include_str!("../locales/languages.json"))
            .expect("validated bundled language names")
    });
    names
        .languages
        .get(locale.code())
        .and_then(|catalog| catalog.get(code))
        .map_or(code, String::as_str)
}

pub fn valid_selection(code: &str) -> bool {
    code == "auto"
        || speech_languages()
            .iter()
            .any(|l| l.code == code && l.model_supported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::LOCALES;
    #[test]
    fn lists_engine_languages_and_honors_exclusions_and_model_vocabulary() {
        let languages = speech_languages();
        assert_eq!(
            languages.len(),
            (whisper_rs::get_lang_max_id() + 1) as usize - 2
        );
        assert!(!languages.iter().any(|l| matches!(l.code, "be" | "uk")));
        for code in ["en", "ru", "fr", "de", "es", "it", "ja", "ar", "sw"] {
            assert!(valid_selection(code));
        }
        assert!(!valid_selection("yue"));
        assert!(!valid_selection("not-a-language"));
        for locale in LOCALES {
            for language in &languages {
                assert_ne!(language_name(locale, language.code), language.code);
            }
        }
    }
}
