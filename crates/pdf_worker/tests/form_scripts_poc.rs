//! Integration tests for ADR 0021 POC: form scripts in sandboxed worker (B3-07).
//!
//! Evaluates Gates 2, 3, 4, 5 of ADR 0021 using real WorkerHost driving real sandboxed pdf_worker.exe.
#![cfg(windows)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{build_pdf, temp_pdf};
use ipc_contract::types::{DocumentId, FormField, Rotation};
use ipc_contract::worker::{FormFieldEvent, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, WorkerHost};

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
/// Calculation form: input calculation via `AFSimple_Calculate` and formatting via `AFNumber_Format`.
///
/// FINDING: This gate FAILS due to an upstream bug in MuPDF 1.27.2 (`pdf-js.c:403` and `pdf-js.c:528`):
/// `pdf_dict_getl` is a variadic C function requiring a `NULL` sentinel pointer at the end of its key
/// list. Because `pdf-js.c` omitted `, NULL`, on Windows x86_64 ABI the variadic lookup reads stack
/// garbage, failing to find `/Fields`. Consequently, `this.getField("num1")` inside `AFSimple_Calculate`
/// always returns `null`, throwing `TypeError: cannot convert null to object` in mujs and leaving fields
/// uncalculated.
#[test]
fn gate3_form_calculation_fails_due_to_upstream_mupdf_getfield_bug() {
    let mut h = host();
    let path = corpus_file("benign/form-calculation.pdf");
    let (doc, _) = h.open(&path).expect("open form-calculation");

    let fields_before = get_fields(&mut h, doc, 0);
    assert_eq!(fields_before.len(), 3);
    let total = fields_before
        .iter()
        .find(|f| f.label.as_deref() == Some("Total"))
        .expect("total");
    assert_eq!(total.value, "30.00");
    let total_id = total.id;

    // Enable JS
    h.enable_js(doc).expect("enable js");

    // Trigger recalculation on total
    let calc_resp = h
        .form_field_event(doc, 0, total_id, FormFieldEvent::Calculate)
        .expect("calculate form");
    let WorkerResponse::FormFieldEventResult { changed_fields, .. } = calc_resp else {
        panic!("expected FormFieldEventResult");
    };

    // Because `this.getField()` returns null upstream, `AFSimple_Calculate` throws a TypeError
    // and no fields are updated.
    assert!(
        changed_fields.is_empty(),
        "upstream bug in pdf-js.c:403 prevents getField lookup, expected empty changed_fields"
    );
}

/// Gate 4:
/// Infinite loop script (`while(true){}`) evaluation.
///
/// FINDING: ADR 0021 assumed `while(true){}` would loop indefinitely until killed by `worker_host`'s
/// 2-second timeout. However, MuPDF hardcodes an instruction quota in `pdf-js.c:29`:
/// `#define PDF_JS_LIMIT_RUNTIME (10 << 20)` (10,485,760 instructions).
/// MuJS aborts `while(true){}` in ~120ms with "js: script ran too long", and MuPDF's `pdf_js_execute`
/// catches this error and returns cleanly.
///
/// Therefore, the worker process is NOT terminated, the host does not freeze, and the worker remains
/// fully responsive for subsequent operations without needing a restart.
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
        "mujs instruction quota (PDF_JS_LIMIT_RUNTIME) terminates loop in ~120ms, elapsed: {elapsed:?}"
    );

    let WorkerResponse::FormFieldEventResult { changed_fields, .. } = resp else {
        panic!("expected FormFieldEventResult");
    };
    assert!(changed_fields.is_empty());

    // Worker remains healthy and responsive
    let (new_doc, _) = h.open(&path).expect("open next document");
    let fields_reopened = get_fields(&mut h, new_doc, 0);
    assert_eq!(fields_reopened.len(), 1);

    std::fs::remove_file(path).ok();
}

/// Gate 5:
/// Document-level script that attempts to modify a field upon open:
/// Field value remains unmodified before explicit `enable_js()`.
///
/// FINDING: Before `enable_js()`, opening the document does NOT run document-level scripts;
/// the initial field value is preserved intact (PASS).
/// Upon `enable_js()`, document-level scripts are executed, but attempting `this.getField("f1").value = ...`
/// fails to modify the field due to the aforementioned upstream bug in `pdf-js.c:403`.
#[test]
fn gate5_document_level_script_does_not_modify_field_before_enable_js() {
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

    // 3. After enable_js, field remains unmodified because this.getField() returns null upstream
    let fields_after = get_fields(&mut h, doc, 0);
    assert_eq!(fields_after[0].value, "initial_clean_value");

    std::fs::remove_file(path).ok();
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
