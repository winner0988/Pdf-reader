//! Checks `src-tauri/windows/installer-hooks.nsh` against the constants and invariants the app
//! depends on: AppContainer profile cleanup (REL-01), Explorer image context menu verbs (B5-13),
//! and the PDF file icon (UX-06).
#![cfg(windows)]

use std::collections::{BTreeMap, BTreeSet};

const HOOKS: &str = include_str!("../../../src-tauri/windows/installer-hooks.nsh");
const BUILD_SCRIPT: &str = include_str!("../../../src-tauri/build.rs");
const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff"];
// Numeric Windows LCIDs matching tauri.conf.json languages order ("English", "TradChinese"):
// Tauri includes installerHooks before MUI_LANGUAGE, and NSIS forbids LangString inside a Section.
const REQUIRED_LANG_IDS: [&str; 2] = ["1033", "1028"];
const REQUIRED_LANG_STRINGS: [&str; 4] = [
    "PDF_READER_WEBVIEW2_MISSING",
    "PDF_READER_CREATE_PDF_LABEL",
    "PDF_READER_PROGID_DESCRIPTION",
    "PDF_READER_APP_DESCRIPTION",
];

fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(
            c,
            '\u{3000}'..='\u{303f}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'
        )
    })
}

fn parse_lang_string(line: &str) -> Option<(&str, &str, &str)> {
    let rest = line.trim().strip_prefix("LangString ")?.trim_start();
    let (name, rest) = rest.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    let (lang_id, rest) = rest.split_once(char::is_whitespace)?;
    let value = rest.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        Some((name, lang_id, &value[1..value.len() - 1]))
    } else {
        None
    }
}

fn collect_lang_strings(source: &str) -> BTreeMap<&str, BTreeMap<&str, &str>> {
    let mut map: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with("LangString ") {
            let (name, lang_id, value) = parse_lang_string(trimmed)
                .unwrap_or_else(|| panic!("invalid LangString syntax: {trimmed}"));
            let prev = map.entry(name).or_default().insert(lang_id, value);
            assert!(
                prev.is_none(),
                "duplicate LangString {name} for language {lang_id}"
            );
        }
    }
    map
}

fn extract_lang_refs(line: &str) -> Vec<&str> {
    let mut refs = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("$(") {
        let after = &rest[start + 2..];
        let Some(end) = after.find(')') else {
            break;
        };
        refs.push(&after[..end]);
        rest = &after[end + 1..];
    }
    refs
}
/// The resource id of the PDF file icon in the exe, which `src-tauri/build.rs` puts there.
const PDF_FILE_ICON: &str = "32514";

fn macro_body<'a>(source: &'a str, name: &str) -> &'a str {
    let exact = format!("!macro {name}");
    let with_args = format!("!macro {name} ");
    let mut offset = 0;
    let mut start = None;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == exact || trimmed.starts_with(&with_args) {
            start = Some(offset);
            break;
        }
        offset += line.len();
    }
    let start = start.unwrap_or_else(|| panic!("installer-hooks.nsh must define !macro {name}"));
    let rest = &source[start..];
    let end = rest
        .find("!macroend")
        .unwrap_or_else(|| panic!("!macro {name} must end with !macroend"));
    &rest[..end]
}

#[test]
fn installer_hooks_has_utf8_bom() {
    assert!(
        HOOKS.starts_with('\u{feff}'),
        "installer-hooks.nsh must be saved with a UTF-8 BOM so NSIS does not garble Chinese text"
    );
}

#[test]
fn uninstaller_deletes_the_worker_profile() {
    let call = format!(
        "DeleteAppContainerProfile(w \"{}\")",
        sandbox::WORKER_APP_CONTAINER
    );
    assert!(
        HOOKS.contains(&call),
        "installer-hooks.nsh must contain {call}"
    );
}

#[test]
fn image_context_menu_registers_all_seven_extensions_with_quoted_command() {
    assert!(
        HOOKS.contains("!define PDF_READER_CREATE_PDF_VERB "),
        "verb name must be defined once via !define"
    );

    let register_verb = macro_body(HOOKS, "PDF_READER_REGISTER_IMAGE_VERB");
    assert!(
        register_verb.contains(
            r#"WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}" "MUIVerb" "$(PDF_READER_CREATE_PDF_LABEL)""#
        ),
        "must set MUIVerb on the verb key using LangString"
    );
    assert!(
        register_verb.contains(
            r#"WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}" "Icon" "$INSTDIR\${MAINBINARYNAME}.exe,0""#
        ),
        "must set Icon to the main binary"
    );
    assert!(
        register_verb.contains(
            r#"WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}" "MultiSelectModel" "Player""#
        ),
        "must set MultiSelectModel=Player on the verb key"
    );
    assert!(
        register_verb.contains(
            r#"WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe" --create-pdf-from "%1"'"#
        ),
        "command must quote both the executable path and %1"
    );

    let register = macro_body(HOOKS, "PDF_READER_REGISTER");
    for ext in IMAGE_EXTENSIONS {
        let invocation = format!("!insertmacro PDF_READER_REGISTER_IMAGE_VERB \"{ext}\"");
        assert!(
            register.contains(&invocation),
            "PDF_READER_REGISTER must register .{ext}"
        );
    }
}

#[test]
fn image_context_menu_unregisters_exact_same_verb_keys_and_not_parent_keys() {
    let unregister_verb = macro_body(HOOKS, "PDF_READER_UNREGISTER_IMAGE_VERB");
    assert!(
        unregister_verb.contains(
            r#"DeleteRegKey HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}""#
        ),
        "must delete only our verb key under SystemFileAssociations\\.${{ext}}\\shell"
    );
    // Must never delete the SystemFileAssociations\.ext key or its shell subkey.
    for line in unregister_verb.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("DeleteRegKey ") {
            assert!(
                trimmed.ends_with(r#"\shell\${PDF_READER_CREATE_PDF_VERB}""#),
                "unexpected DeleteRegKey target: {trimmed}"
            );
        }
    }

    let unregister = macro_body(HOOKS, "PDF_READER_UNREGISTER");
    for ext in IMAGE_EXTENSIONS {
        let invocation = format!("!insertmacro PDF_READER_UNREGISTER_IMAGE_VERB \"{ext}\"");
        assert!(
            unregister.contains(&invocation),
            "PDF_READER_UNREGISTER must unregister .{ext}"
        );
    }
}

#[test]
fn installer_never_writes_user_choice_or_extension_default_values() {
    for line in HOOKS.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(';') {
            continue;
        }
        assert!(
            !trimmed.to_ascii_lowercase().contains("userchoice"),
            "installer-hooks.nsh must never touch UserChoice: {trimmed}"
        );
        if trimmed.starts_with("WriteReg") {
            for ext in IMAGE_EXTENSIONS.iter().chain(&["pdf"]) {
                // Must not set the default value ("") of Software\Classes\.<ext>
                let direct_ext = format!(r#"Software\Classes\.{ext}""#);
                assert!(
                    !trimmed.contains(&direct_ext),
                    "must not write to extension key directly: {trimmed}"
                );
                // Must not touch OpenWithProgids for image extensions.
                if *ext != "pdf" {
                    let open_with = format!(r#"Software\Classes\.{ext}\"#);
                    assert!(
                        !trimmed.contains(&open_with),
                        "must not touch .{ext} ProgIDs or OpenWithProgids: {trimmed}"
                    );
                }
            }
            // Must not set the default value ("") of SystemFileAssociations\.${ext}
            assert!(
                !trimmed.contains(r#"SystemFileAssociations\.${ext}""#),
                "must not write to SystemFileAssociations\\.ext default value: {trimmed}"
            );
        }
    }
}

#[test]
fn every_lang_string_is_defined_in_both_languages_at_top_level() {
    let lang_strings = collect_lang_strings(HOOKS);
    for required in REQUIRED_LANG_STRINGS {
        assert!(
            lang_strings.contains_key(required),
            "installer-hooks.nsh must define LangString {required}"
        );
    }

    let mut in_macro = false;
    for line in HOOKS.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("!macro ") {
            in_macro = true;
        } else if trimmed == "!macroend" {
            in_macro = false;
        } else if trimmed.starts_with("LangString ") {
            assert!(
                !in_macro,
                "LangString must be at top level outside hook macros (NSIS forbids LangString inside Section): {trimmed}"
            );
        }
    }

    for (name, entries) in &lang_strings {
        for lang_id in REQUIRED_LANG_IDS {
            let value = entries.get(lang_id).unwrap_or_else(|| {
                panic!("LangString {name} is missing entry for {lang_id}");
            });
            assert!(
                !value.trim().is_empty(),
                "LangString {name} for {lang_id} must not be empty"
            );
            if lang_id == "1033" {
                assert!(
                    !has_cjk(value),
                    "LangString {name} for {lang_id} must not contain Chinese characters: {value}"
                );
            } else {
                assert!(
                    has_cjk(value),
                    "LangString {name} for {lang_id} must contain Chinese characters: {value}"
                );
            }
        }
        let expected_keys: BTreeSet<&str> = REQUIRED_LANG_IDS.into_iter().collect();
        let actual_keys: BTreeSet<&str> = entries.keys().copied().collect();
        assert_eq!(
            actual_keys, expected_keys,
            "LangString {name} must define only English (1033) and TradChinese (1028)"
        );
    }
}

#[test]
fn message_boxes_and_registry_strings_use_lang_strings_without_hardcoded_chinese() {
    let lang_strings = collect_lang_strings(HOOKS);
    let mut used_refs = BTreeSet::new();

    for line in HOOKS.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with("MessageBox ") || trimmed.starts_with("WriteRegStr ") {
            assert!(
                !has_cjk(trimmed),
                "MessageBox / WriteRegStr must use $(LangString) rather than hardcoded Chinese: {trimmed}"
            );
            for lang_ref in extract_lang_refs(trimmed) {
                assert!(
                    lang_strings.contains_key(lang_ref),
                    "referenced $({lang_ref}) has no LangString definition: {trimmed}"
                );
                used_refs.insert(lang_ref);
            }
        }
    }

    let preinstall = macro_body(HOOKS, "NSIS_HOOK_PREINSTALL");
    assert!(
        preinstall.contains(
            r#"MessageBox MB_OK|MB_ICONEXCLAMATION "$(PDF_READER_WEBVIEW2_MISSING)" /SD IDOK"#
        ),
        "NSIS_HOOK_PREINSTALL must show $(PDF_READER_WEBVIEW2_MISSING)"
    );

    let register = macro_body(HOOKS, "PDF_READER_REGISTER");
    assert!(
        register.contains(
            r#"WriteRegStr HKLM "Software\Classes\${PDF_READER_PROGID}" "" "$(PDF_READER_PROGID_DESCRIPTION)""#
        ),
        "PDF_READER_REGISTER must write $(PDF_READER_PROGID_DESCRIPTION)"
    );
    assert!(
        register.contains(
            r#"WriteRegStr HKLM "${PDF_READER_CAPABILITIES}" "ApplicationDescription" "$(PDF_READER_APP_DESCRIPTION)""#
        ),
        "PDF_READER_REGISTER must write $(PDF_READER_APP_DESCRIPTION)"
    );

    let expected_used: BTreeSet<&str> = REQUIRED_LANG_STRINGS.into_iter().collect();
    assert_eq!(
        used_refs, expected_used,
        "all required LangStrings must be referenced in MessageBox or WriteRegStr"
    );
}

#[test]
fn pdf_files_show_the_file_icon_that_build_rs_puts_in_the_exe() {
    let register = macro_body(HOOKS, "PDF_READER_REGISTER");
    let default_icon = format!(
        r#"WriteRegStr HKLM "Software\Classes\${{PDF_READER_PROGID}}\DefaultIcon" "" "$INSTDIR\${{MAINBINARYNAME}}.exe,-{PDF_FILE_ICON}""#
    );
    assert!(
        register.contains(&default_icon),
        "the ProgID's DefaultIcon must be resource {PDF_FILE_ICON} of the exe, the PDF file icon"
    );
    let resource = format!(r#"{PDF_FILE_ICON} ICON \"icons/pdf-file.ico\""#);
    assert!(
        BUILD_SCRIPT.contains(&resource),
        "build.rs must put icons/pdf-file.ico in the exe as resource {PDF_FILE_ICON}"
    );
    // The Default apps page of Windows Settings stands for the app, so it keeps the app icon.
    assert!(
        register.contains(
            r#"WriteRegStr HKLM "${PDF_READER_CAPABILITIES}" "ApplicationIcon" "$INSTDIR\${MAINBINARYNAME}.exe,0""#
        ),
        "the Default apps page must keep the app icon"
    );
}
