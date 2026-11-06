//! The wrap-up's two preferences (specs/0083 W2a): summarize automatically, and the pinned
//! summary language. Both live in `RecordingPreferences` and are mirrored here on every load
//! and save, so the wrap-up never waits on the store (the `power::set_low_power_preference`
//! pattern).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use serde::{Deserialize, Deserializer, Serialize};
use tauri::{AppHandle, Runtime};

use crate::audio::recording_preferences::{
    load_recording_preferences, save_recording_preferences, RecordingPreferences,
};

static AUTO_SUMMARY: AtomicBool = AtomicBool::new(true);
static LANGUAGE_DEFAULT: RwLock<Option<String>> = RwLock::new(None);

/// Copy the two preferences out of a loaded or saved `RecordingPreferences`.
pub fn mirror(prefs: &RecordingPreferences) {
    AUTO_SUMMARY.store(
        prefs.auto_summary_after_recording.unwrap_or(true),
        Ordering::SeqCst,
    );
    *LANGUAGE_DEFAULT.write().unwrap_or_else(|e| e.into_inner()) =
        prefs.summary_language_default.clone();
}

pub fn auto_summary() -> bool {
    AUTO_SUMMARY.load(Ordering::SeqCst)
}

pub fn summary_language_default() -> Option<String> {
    LANGUAGE_DEFAULT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WrapUpPreferences {
    pub auto_summary: bool,
    pub summary_language_default: Option<String>,
}

/// A partial update. An absent field is left alone; `summaryLanguageDefault: null` clears it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WrapUpPreferencesPatch {
    #[serde(default)]
    pub auto_summary: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    pub summary_language_default: Option<Option<String>>,
}

fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

/// Apply `patch` to `prefs`. A language code must be one the summarizer supports.
pub fn apply_patch(
    prefs: &mut RecordingPreferences,
    patch: WrapUpPreferencesPatch,
) -> Result<(), String> {
    if let Some(on) = patch.auto_summary {
        prefs.auto_summary_after_recording = Some(on);
    }
    if let Some(language) = patch.summary_language_default {
        prefs.summary_language_default = match language.filter(|l| !l.trim().is_empty()) {
            Some(code) => Some(
                crate::summary::metadata::normalise_supported_summary_language(&code)
                    .map_err(|e| format!("{e}. Choose a language from the list."))?,
            ),
            None => None,
        };
    }
    Ok(())
}

fn view(prefs: &RecordingPreferences) -> WrapUpPreferences {
    WrapUpPreferences {
        auto_summary: prefs.auto_summary_after_recording.unwrap_or(true),
        summary_language_default: prefs.summary_language_default.clone(),
    }
}

#[tauri::command]
pub async fn api_get_wrap_up_preferences<R: Runtime>(
    app: AppHandle<R>,
) -> Result<WrapUpPreferences, String> {
    load_recording_preferences(&app)
        .await
        .map(|p| view(&p))
        .map_err(|e| format!("Couldn't read your recording settings: {e}"))
}

#[tauri::command]
pub async fn api_set_wrap_up_preferences<R: Runtime>(
    app: AppHandle<R>,
    preferences: WrapUpPreferencesPatch,
) -> Result<WrapUpPreferences, String> {
    let mut prefs = load_recording_preferences(&app)
        .await
        .map_err(|e| format!("Couldn't read your recording settings: {e}"))?;
    apply_patch(&mut prefs, preferences)?;
    save_recording_preferences(&app, &prefs)
        .await
        .map_err(|e| format!("Couldn't save your recording settings: {e}"))?;
    Ok(view(&prefs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(json: serde_json::Value) -> WrapUpPreferencesPatch {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn auto_summary_defaults_on_when_unset() {
        assert!(view(&RecordingPreferences::default()).auto_summary);
    }

    #[test]
    fn an_absent_field_is_left_alone_and_null_clears_the_language() {
        let mut p = RecordingPreferences {
            summary_language_default: Some("fr".into()),
            ..Default::default()
        };
        apply_patch(&mut p, patch(serde_json::json!({ "autoSummary": false }))).unwrap();
        assert_eq!(p.auto_summary_after_recording, Some(false));
        assert_eq!(p.summary_language_default.as_deref(), Some("fr"));

        apply_patch(
            &mut p,
            patch(serde_json::json!({ "summaryLanguageDefault": null })),
        )
        .unwrap();
        assert_eq!(p.summary_language_default, None);
        assert_eq!(p.auto_summary_after_recording, Some(false));
    }

    #[test]
    fn a_language_is_normalised_and_an_unsupported_one_refused() {
        let mut p = RecordingPreferences::default();
        apply_patch(
            &mut p,
            patch(serde_json::json!({ "summaryLanguageDefault": "DE_de" })),
        )
        .unwrap();
        assert_eq!(p.summary_language_default.as_deref(), Some("de"));
        assert!(apply_patch(
            &mut p,
            patch(serde_json::json!({ "summaryLanguageDefault": "xx-nope" }))
        )
        .is_err());
        assert_eq!(p.summary_language_default.as_deref(), Some("de"));
    }
}
