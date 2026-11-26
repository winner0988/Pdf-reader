//! Fuzz target for form field events execution via MuPDF and mujs (ADR 0021, Gate 6).
//!
//! Feeds arbitrary PDF bytes to MuPDF, enables JavaScript, and dispatches Keystroke, Format,
//! Validate, and Calculate events against discovered form fields.
#![no_main]

use ipc_contract::worker::FormFieldEvent;
use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::PdfDocument;

fuzz_target!(|data: &[u8]| {
    let Ok(mut document) = PdfDocument::from_bytes(data) else {
        return;
    };
    // Gate 1 limit: scripts over 1 MB are rejected; enable_js handles this safely.
    if document.enable_js().is_err() {
        return;
    }

    let Ok(pages) = document.page_count() else {
        return;
    };

    let sample_change = if data.len() >= 4 {
        String::from_utf8_lossy(&data[..data.len().min(32)]).into_owned()
    } else {
        "1".to_string()
    };

    for page_index in 0..pages.min(3) {
        let Ok(fields) = document.page_fields(page_index) else {
            continue;
        };
        for field in fields.into_iter().take(5) {
            let _ = document.dispatch_field_event(
                page_index,
                field.id,
                &FormFieldEvent::Keystroke {
                    value: "100".into(),
                    change: sample_change.clone(),
                    sel_start: 0,
                    sel_end: 0,
                    will_commit: false,
                },
            );
            let _ = document.dispatch_field_event(
                page_index,
                field.id,
                &FormFieldEvent::Format,
            );
            let _ = document.dispatch_field_event(
                page_index,
                field.id,
                &FormFieldEvent::Validate {
                    value: sample_change.clone(),
                },
            );
            let _ = document.dispatch_field_event(
                page_index,
                field.id,
                &FormFieldEvent::Calculate,
            );
        }
    }
});
