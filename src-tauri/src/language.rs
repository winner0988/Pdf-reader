//! Choice of the interface language (ADR 0029, #683).
//!
//! The main process determines the interface language once at startup:
//! - If the setting is `system`, it queries Windows for the user's preferred UI language via
//!   `GetUserPreferredUILanguages`. Chinese in any region maps to `zh-TW`; all other languages map
//!   to `en`.
//! - If the setting is `zh-TW` or `en`, it uses that choice directly.
//!
//! The effective language and the detected system language are handed to the WebView through Tauri's
//! initialization script (`window.__PDF_READER_LANGUAGE__` and `window.__PDF_READER_SYSTEM_LANGUAGE__`).

#![allow(unsafe_code)]

use ipc_contract::types::LanguagePreference;

/// Maps a Windows language name (BCP 47 tag, e.g. "zh-TW", "en-US") to an app language.
pub fn map_language_name(lang_name: &str) -> &'static str {
    let lower = lang_name.trim().to_ascii_lowercase();
    if lower.starts_with("zh") {
        "zh-TW"
    } else {
        "en"
    }
}

/// The system's preferred UI language according to Windows.
#[cfg(windows)]
pub fn system_ui_language() -> &'static str {
    if let Ok(val) = std::env::var("PDF_READER_SYSTEM_LANGUAGE") {
        return map_language_name(&val);
    }

    use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

    let mut num_languages: u32 = 0;
    let mut buffer_size: u32 = 0;
    // SAFETY: Passing a null pointer for the buffer with `buffer_size` initialized to 0 queries
    // the required buffer size in UTF-16 code units; `num_languages` and `buffer_size` are valid
    // out-pointers to stack variables.
    let success = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut num_languages,
            std::ptr::null_mut(),
            &mut buffer_size,
        )
    };
    if success == 0 || buffer_size == 0 || num_languages == 0 {
        return "en";
    }

    let mut buffer = vec![0u16; buffer_size as usize];
    // SAFETY: `buffer.as_mut_ptr()` comes from the allocated vector `buffer` of length `buffer_size`,
    // which lives through the call; `buffer_size` passes its capacity and receives the written code
    // units. If the call fails (`success == 0`), `buffer` is not read and we return early.
    let success = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut num_languages,
            buffer.as_mut_ptr(),
            &mut buffer_size,
        )
    };
    if success == 0 {
        return "en";
    }

    // The buffer is a sequence of null-terminated UTF-16 strings terminated by an extra null.
    let first_len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let first_lang = String::from_utf16_lossy(&buffer[..first_len]);
    map_language_name(&first_lang)
}

#[cfg(not(windows))]
pub fn system_ui_language() -> &'static str {
    "zh-TW"
}

/// Resolves the effective language and the detected system language.
///
/// Both returned strings are strictly either `"zh-TW"` or `"en"`.
pub fn resolve_language(preference: LanguagePreference) -> (&'static str, &'static str) {
    let sys = system_ui_language();
    let effective = match preference {
        LanguagePreference::System => sys,
        LanguagePreference::ZhTw => "zh-TW",
        LanguagePreference::En => "en",
    };
    (effective, sys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_chinese_tags_to_zh_tw() {
        for tag in [
            "zh-TW", "zh-CN", "zh-HK", "zh-Hans", "zh-Hant", "zh-MO", "ZH-tw", "zh",
        ] {
            assert_eq!(map_language_name(tag), "zh-TW");
        }
    }

    #[test]
    fn maps_other_languages_to_english() {
        for tag in [
            "en-US", "en-GB", "ja-JP", "ko-KR", "de-DE", "fr-FR", "es-ES", "", "unknown",
        ] {
            assert_eq!(map_language_name(tag), "en");
        }
    }

    #[test]
    fn explicit_preference_overrides_system() {
        let (effective_tw, _) = resolve_language(LanguagePreference::ZhTw);
        assert_eq!(effective_tw, "zh-TW");

        let (effective_en, _) = resolve_language(LanguagePreference::En);
        assert_eq!(effective_en, "en");
    }

    #[test]
    fn system_preference_uses_detected_system_language() {
        let (effective, sys) = resolve_language(LanguagePreference::System);
        assert_eq!(effective, sys);
        assert!(effective == "zh-TW" || effective == "en");
    }
}
