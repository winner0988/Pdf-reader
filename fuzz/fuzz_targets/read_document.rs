//! Reading an untrusted PDF with MuPDF: annotations, form fields, text boxes, page text,
//! personal data scan, redactions, attachments, signature fields, and opening with passwords
//! (QA-03, #426). Split from `open_document` to keep both fast.
#![no_main]

use std::io::Write;

use ipc_contract::types::{FindPersonalDataQuery, PersonalDataCategory};
use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::{EngineError, PdfDocument};

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
    for index in 0..pages.min(2) {
        if let Ok(annotations) = document.page_annotations(index) {
            for annotation in annotations {
                let _ = document.text_box(index, annotation.id);
            }
        }
        let _ = document.page_fields(index);
        let _ = document.page_text(index, 100_000);
    }
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
    let _ = document.document_properties();
    let _ = document.document_fonts();
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
