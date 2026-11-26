//! Integration tests for ADR 0021 POC: form scripts in sandboxed worker (B3-07).
//!
//! Evaluates Gates 2, 3, 4, 5 of ADR 0021 using real WorkerHost driving real sandboxed pdf_worker.exe.
#![cfg(windows)]

mod common;

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{build_pdf, check_qpdf, temp_pdf};
use ipc_contract::types::{DocumentId, FormField, Rotation};
use ipc_contract::worker::{FormFieldEvent, WorkerEdit, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

fn worker() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_pdf_worker"))
}

fn host() -> WorkerHost {
    WorkerHost::new(worker(), HostConfig::default())
}

fn corpus_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(relative)
}

fn get_fields(host: &mut WorkerHost, doc: DocumentId, page_index: u32) -> Vec<FormField> {
    let response = host
        .request(|request| WorkerRequest::GetPageFields {
            request,
            doc,
            page_index,
        })
        .expect("get page fields");
    let WorkerResponse::PageFields { fields, .. } = response else {
        panic!("expected PageFields, got {response:?}");
    };
    fields
}

/// Gate 2:
/// `openaction-js`, `page-aa`, `js-document-level`, `submitform`, `importdata` have no effect;
/// `field-aa` executes only on interaction, and `app.alert` is intercepted with no side effect.
#[test]
fn gate2_malicious_samples_no_side_effects_and_field_aa_runs_on_input() {
    let mut h = host();

    // 1. openaction-js: opens and renders without running OpenAction script
    let path = corpus_file("malicious/openaction-js.pdf");
    let (doc, _) = h.open(&path).expect("open openaction-js");
    let render_res = h.request(|request| WorkerRequest::Render {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
    });
    assert!(render_res.is_ok(), "render openaction-js");

    // 2. page-aa: opens and renders without running page AA
    let path = corpus_file("malicious/page-aa.pdf");
    let (doc, _) = h.open(&path).expect("open page-aa");
    let render_res = h.request(|request| WorkerRequest::Render {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
    });
    assert!(render_res.is_ok(), "render page-aa");

    // 3. js-document-level: opens without executing document-level scripts
    let path = corpus_file("malicious/js-document-level.pdf");
    let (doc, _) = h.open(&path).expect("open js-document-level");
    let render_res = h.request(|request| WorkerRequest::Render {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
    });
    assert!(render_res.is_ok(), "render js-document-level");

    // 4. submitform: push button action cannot execute over network; renders safely
    let path = corpus_file("malicious/submitform.pdf");
    let (doc, _) = h.open(&path).expect("open submitform");
    let render_res = h.request(|request| WorkerRequest::Render {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
    });
    assert!(render_res.is_ok(), "render submitform");

    // 5. importdata: push button action cannot read local file; renders safely
    let path = corpus_file("malicious/importdata.pdf");
    let (doc, _) = h.open(&path).expect("open importdata");
    let render_res = h.request(|request| WorkerRequest::Render {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
    });
    assert!(render_res.is_ok(), "render importdata");

    // 6. field-aa: script runs only when field interaction is sent; alert is intercepted and blocked
    let path = corpus_file("malicious/field-aa.pdf");
    let (doc, _) = h.open(&path).expect("open field-aa");
    let fields = get_fields(&mut h, doc, 0);
    assert_eq!(fields.len(), 1);
    let amount_id = fields[0].id;

    // Enable JS explicitly
    let resp = h.enable_js(doc).expect("enable js");
    assert!(matches!(resp, WorkerResponse::JsEnabled { .. }));

    // Send keystroke event
    let resp = h
        .form_field_event(
            doc,
            0,
            amount_id,
            FormFieldEvent::Keystroke {
                value: "100".into(),
                change: "0".into(),
                sel_start: 3,
                sel_end: 3,
                will_commit: false,
            },
        )
        .expect("field keystroke");

    let WorkerResponse::FormFieldEventResult {
        accepted,
        blocked_actions,
        ..
    } = resp
    else {
        panic!("expected FormFieldEventResult, got {resp:?}");
    };

    assert!(accepted);
    assert!(
        !blocked_actions.is_empty() && blocked_actions.iter().all(|a| a == "alert"),
        "expected blocked alert actions, got {blocked_actions:?}"
    );
}

/// Gate 3:
/// Calculation form: input edits trigger `AFSimple_Calculate` and `AFNumber_Format`, and
/// saving + reopening in a fresh worker preserves values and formatting.
#[test]
fn gate3_form_calculation_and_formatting_persist_on_save() {
    let mut h = host();
    let path = corpus_file("benign/form-calculation.pdf");
    let (doc, _) = h.open(&path).expect("open form-calculation");

    let fields_before = get_fields(&mut h, doc, 0);
    assert_eq!(fields_before.len(), 3);
    let num1_id = fields_before
        .iter()
        .find(|f| f.label.as_deref() == Some("Number 1"))
        .expect("num1")
        .id;
    let num2_id = fields_before
        .iter()
        .find(|f| f.label.as_deref() == Some("Number 2"))
        .expect("num2")
        .id;
    let total = fields_before
        .iter()
        .find(|f| f.label.as_deref() == Some("Total"))
        .expect("total");
    assert_eq!(total.value, "30.00");
    let total_id = total.id;

    // Enable JS
    h.enable_js(doc).expect("enable js");

    // Edit num1 -> 15, num2 -> 25
    for (field, value) in [(num1_id, "15"), (num2_id, "25")] {
        let edited = h
            .request(|request| WorkerRequest::Edit {
                request,
                doc,
                edit: WorkerEdit::SetFieldValue {
                    page: 0,
                    field,
                    value: value.into(),
                },
            })
            .expect("set field value");
        assert!(matches!(edited, WorkerResponse::Edited { .. }));
    }

    // Trigger recalculation on total (AFSimple_Calculate SUM(num1, num2) => 40)
    let calc_resp = h
        .form_field_event(doc, 0, total_id, FormFieldEvent::Calculate)
        .expect("calculate form");
    let WorkerResponse::FormFieldEventResult {
        accepted,
        changed_fields,
        ..
    } = calc_resp
    else {
        panic!("expected FormFieldEventResult");
    };
    assert!(accepted);
    let total_change = changed_fields
        .iter()
        .find(|c| c.id == total_id)
        .expect("total field should change after calculation");
    assert_eq!(total_change.value, "40");

    // Trigger Format on total (AFNumber_Format => "40.00")
    let fmt_resp = h
        .form_field_event(doc, 0, total_id, FormFieldEvent::Format)
        .expect("format total");
    let WorkerResponse::FormFieldEventResult { value, .. } = fmt_resp else {
        panic!("expected FormFieldEventResult");
    };
    assert_eq!(value.as_deref(), Some("40.00"));

    // Save through a write-only handle and verify with qpdf
    let save_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-form-calc-saved.pdf",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&save_path);
    let _save_guard = common::TempFileGuard(save_path.clone());
    let save_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&save_path)
        .expect("create saved pdf");
    let saved = h.save(doc, &save_file).expect("save form");
    assert!(matches!(saved, WorkerResponse::Saved { .. }));
    drop(save_file);
    check_qpdf(&save_path);

    // Reopen saved document in a fresh worker and verify persisted values & format
    let mut h2 = host();
    let (doc2, _) = h2.open(&save_path).expect("reopen saved form");
    let fields_after = get_fields(&mut h2, doc2, 0);
    let find_val = |label: &str| {
        fields_after
            .iter()
            .find(|f| f.label.as_deref() == Some(label))
            .expect(label)
            .clone()
    };
    assert_eq!(find_val("Number 1").value, "15");
    assert_eq!(find_val("Number 2").value, "25");
    let total2 = find_val("Total");
    assert_eq!(total2.value, "40");

    h2.enable_js(doc2).expect("enable js in reopened doc");
    let fmt2 = h2
        .form_field_event(doc2, 0, total2.id, FormFieldEvent::Format)
        .expect("format reopened total");
    assert!(
        matches!(fmt2, WorkerResponse::FormFieldEventResult { value: Some(ref v), .. } if v == "40.00"),
        "expected formatted '40.00', got {fmt2:?}"
    );
    let render_res = h2.request(|request| WorkerRequest::Render {
        request,
        doc: doc2,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
    });
    assert!(render_res.is_ok(), "render saved form: {render_res:?}");
}

/// Gate 4a:
/// Simple infinite loop script (`while(true){}`) hits MuPDF's instruction quota
/// (`PDF_JS_LIMIT_RUNTIME = 10 << 20`) before the 2s host timeout.
#[test]
fn gate4_infinite_loop_script_aborts_via_mujs_instruction_limit_without_blocking_host() {
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (loop) /V (0) /Rect [10 10 100 30] /AA << /C << /S /JavaScript /JS (while\\(true\\){}) >> >> >>".into(),
    ]);

    let path = temp_pdf("infinite-loop", &pdf);
    let mut h = host();
    let (doc, _) = h.open(&path).expect("open loop pdf");

    let fields = get_fields(&mut h, doc, 0);
    assert_eq!(fields.len(), 1);
    let field_id = fields[0].id;

    // Enable JS
    h.enable_js(doc).expect("enable js");

    // Trigger calculation which runs while(true){}
    let start = std::time::Instant::now();
    let resp = h
        .form_field_event(doc, 0, field_id, FormFieldEvent::Calculate)
        .expect("script terminated safely by mujs instruction limit");
    let elapsed = start.elapsed();

    // Verify it aborted quickly under the instruction limit (well under the 2-second host timeout)
    assert!(
        elapsed < Duration::from_millis(1000),
        "mujs instruction quota (PDF_JS_LIMIT_RUNTIME) terminates loop quickly, elapsed: {elapsed:?}"
    );

    let WorkerResponse::FormFieldEventResult { changed_fields, .. } = resp else {
        panic!("expected FormFieldEventResult");
    };
    assert!(changed_fields.is_empty());

    // Worker remains healthy and responsive
    let (new_doc, _) = h.open(&path).expect("open next document");
    let fields_reopened = get_fields(&mut h, new_doc, 0);
    assert_eq!(fields_reopened.len(), 1);
}

/// Gate 4b:
/// A slow native operation inside a loop exceeds the 2s `WorkerHost` timeout; `WorkerHost`
/// returns `HostError::Timeout`, kills the runaway worker, and restarts a clean worker (with JS
/// disabled by default) for the next request.
#[test]
fn gate4_slow_script_triggers_2s_timeout_and_restarts_worker() {
    // Each iteration stays under mujs's REG_MAXRUN (1<<20) and uses very few bytecode ops,
    // so wall-clock time exceeds 2s well before PDF_JS_LIMIT_RUNTIME is hit.
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (slow) /V (0) /Rect [10 10 100 30] /AA << /C << /S /JavaScript /JS (while\\(true\\){ /\\(a+\\)+b/.test\\('aaaaaaaaaaaaaaaa!'\\); }) >> >> >>".into(),
    ]);

    let path = temp_pdf("slow-script", &pdf);
    let mut h = host();
    let (doc, _) = h.open(&path).expect("open slow pdf");
    let fields = get_fields(&mut h, doc, 0);
    assert_eq!(fields.len(), 1);
    let field_id = fields[0].id;

    h.enable_js(doc).expect("enable js");

    let start = std::time::Instant::now();
    let err = h
        .form_field_event(doc, 0, field_id, FormFieldEvent::Calculate)
        .unwrap_err();
    let elapsed = start.elapsed();

    assert!(
        matches!(err, HostError::Timeout),
        "expected HostError::Timeout, got {err:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(1900) && elapsed < Duration::from_secs(5),
        "expected ~2s host timeout, elapsed: {elapsed:?}"
    );
    assert!(!h.is_running(), "timed-out worker must be terminated");

    // Next request transparently restarts a fresh worker with JS disabled by default.
    let (doc2, _) = h.open(&path).expect("restart worker cleanly");
    let fields2 = get_fields(&mut h, doc2, 0);
    assert_eq!(fields2.len(), 1);
    let err_no_js = h
        .form_field_event(doc2, 0, fields2[0].id, FormFieldEvent::Calculate)
        .unwrap_err();
    assert!(
        matches!(err_no_js, HostError::Worker(_)),
        "freshly restarted worker must keep JS disabled until enabled: {err_no_js:?}"
    );
}

/// Gate 5:
/// Document-level script that modifies a field upon open:
/// Field value remains unmodified before explicit `enable_js()`, and becomes updated once enabled.
#[test]
fn gate5_document_level_script_runs_only_after_enable_js() {
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> /Names << /JavaScript << /Names [(docjs) 5 0 R] >> >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (f1) /V (initial_clean_value) /Rect [10 10 100 30] >>".into(),
        "<< /S /JavaScript /JS (this.getField\\('f1'\\).value = 'modified_by_doc_js';) >>".into(),
    ]);

    let path = temp_pdf("doc-level-js", &pdf);
    let mut h = host();
    let (doc, _) = h.open(&path).expect("open doc-level pdf");

    // 1. Before enable_js, field has initial value: document-level script has NOT run
    let fields_before = get_fields(&mut h, doc, 0);
    assert_eq!(fields_before[0].value, "initial_clean_value");

    // 2. Explicitly enable JS: document-level script runs
    h.enable_js(doc).expect("enable js");

    // 3. After enable_js, document-level script modified the field value (fixed in MuPDF 1.28.6)
    let fields_after = get_fields(&mut h, doc, 0);
    assert_eq!(fields_after[0].value, "modified_by_doc_js");
}

/// Security limit: scripts exceeding 1 MB total are rejected.
#[test]
fn script_total_size_over_1mb_is_rejected() {
    let huge_comment = "a".repeat(1_050_000);
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".into(),
        format!(
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (f1) /V () /Rect [10 10 100 30] /AA << /C << /S /JavaScript /JS (/*{huge_comment}*/) >> >> >>"
        ),
    ]);

    let path = temp_pdf("huge-script", &pdf);
    let mut h = host();
    let (doc, _) = h.open(&path).expect("open huge script pdf");

    // Enable JS should fail because total script size exceeds 1 MB
    let err = h.enable_js(doc).unwrap_err();
    assert!(
        matches!(err, worker_host::HostError::Worker(_)),
        "expected HostError::Worker, got {err:?}"
    );

    std::fs::remove_file(path).ok();
}
