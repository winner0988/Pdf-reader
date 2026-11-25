//! Checks `src-tauri/windows/installer-hooks.nsh` against the constants and invariants the app
//! depends on: AppContainer profile cleanup (REL-01) and Explorer image context menu verbs (B5-13).
#![cfg(windows)]

const HOOKS: &str = include_str!("../../../src-tauri/windows/installer-hooks.nsh");
const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff"];

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
            r#"WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}" "MUIVerb" "${PDF_READER_CREATE_PDF_LABEL}""#
        ),
        "must set MUIVerb on the verb key"
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
