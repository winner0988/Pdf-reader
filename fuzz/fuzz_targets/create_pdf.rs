//! Making a PDF of an untrusted file with MuPDF (B3-03): a picture, a web page, a text, a book,
//! an XPS file, an SVG picture or a Word, Excel or PowerPoint file. The first byte says which
//! kind, the rest is the file. The sandbox contains a crash (ADR 0008), but it does not replace
//! finding it.
#![no_main]

use ipc_contract::types::CreateKind;
use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::PdfBuilder;

/// In this order: the seeds in `.github/workflows/fuzz.yml` start with the number of their kind.
const KINDS: [CreateKind; 9] = [
    CreateKind::Image,
    CreateKind::Html,
    CreateKind::Text,
    CreateKind::Epub,
    CreateKind::Xps,
    CreateKind::Svg,
    CreateKind::Docx,
    CreateKind::Xlsx,
    CreateKind::Pptx,
];

fuzz_target!(|data: &[u8]| {
    let Some((&first, file)) = data.split_first() else {
        return;
    };
    let mut builder = PdfBuilder::new();
    if builder
        .add(file, KINDS[usize::from(first) % KINDS.len()])
        .is_err()
    {
        return;
    }
    // Pictures go together: the same one again is a second page.
    if first as usize % KINDS.len() == 0 {
        let _ = builder.add(file, CreateKind::Image);
    }
    let mut out = Vec::new();
    let _ = builder.finish(&mut out);
});
