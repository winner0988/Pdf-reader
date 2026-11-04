//! Opening an untrusted PDF with MuPDF, then everything the worker does with it (QA-03):
//! page sizes, rendering, the outline, links, the active-content scan, search, annotations,
//! fields, text boxes, page text, personal data scan, redactions, attachments, signatures
//! and the Word document made of the first pages (B3-02). The sandbox
//! contains a crash (ADR 0008), but it does not replace finding it.
#![no_main]

use std::io::Write;
use std::time::Duration;

use ipc_contract::types::{FindPersonalDataQuery, PersonalDataCategory};
use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::{EngineError, PdfDocument};
use pdf_worker::scan::ScanBudget;

struct LimitedWriter<W> {
    inner: W,
    written: usize,
    limit: usize,
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.written.saturating_add(buf.len()) > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "attachment output limit exceeded",
            ));
        }
        let bytes = self.inner.write(buf)?;
        self.written = self.written.saturating_add(bytes);
        Ok(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn exercise_document(document: &PdfDocument) {
    let Ok(pages) = document.page_count() else {
        return;
    };
    // The first pages are enough: they exercise the same code, and each run stays fast.
    for index in 0..pages.min(2) {
        let _ = document.page_size(index);
        // Small scales keep rasters tiny; 90° exercises the rotated path.
        let _ = document.render(index, 0.1, 90);
        let _ = document.page_links(index, 100);
        let _ = document.search_page(index, "a", false, 100);
        if let Ok(annotations) = document.page_annotations(index) {
            for annotation in annotations {
                let _ = document.text_box(index, annotation.id);
            }
        }
        let _ = document.page_fields(index);
        let _ = document.page_text(index, 100_000);
    }
    // The Word document of the same pages (B3-02): their text as XML, their pictures as PNG files.
    let first_pages: Vec<u32> = (0..pages.min(2)).collect();
    let _ = document.docx(&first_pages, &mut std::io::sink());
    let _ = document.outline(1_000, 64);
    let _ = document.has_outline();
    let _ = document.active_content(ScanBudget {
        max_objects: 100_000,
        max_time: Duration::from_secs(1),
    });
    let _ = document.annotation_summary();
    let query = FindPersonalDataQuery {
        categories: vec![
            PersonalDataCategory::TaiwanId,
            PersonalDataCategory::MobilePhone,
            PersonalDataCategory::LandlinePhone,
            PersonalDataCategory::Email,
            PersonalDataCategory::CreditCard,
            PersonalDataCategory::UnifiedBusinessNumber,
            PersonalDataCategory::Keyword,
        ],
        keyword: Some("a".to_string()),
    };
    let _ = document.find_personal_data(&query);
    let _ = document.redaction_summary();
    let attachments = document.list_attachments();
    for attachment in attachments.iter().take(2) {
        let mut writer = LimitedWriter {
            inner: std::io::sink(),
            written: 0,
            limit: 1024 * 1024,
        };
        let _ = document.write_attachment(attachment.id, &mut writer);
    }
    if let Ok(signatures) = document.signature_fields() {
        for field in &signatures.fields {
            let _ = field.contents();
        }
    }
}

fuzz_target!(|data: &[u8]| {
    match PdfDocument::open(data, None) {
        Ok(document) => {
            exercise_document(&document);
        }
        Err(EngineError::Encrypted) => {
            for password in ["user", "owner", "wrong"] {
                if let Ok(document) = PdfDocument::open(data, Some(password)) {
                    exercise_document(&document);
                }
            }
        }
        Err(_) => {}
    }
});
