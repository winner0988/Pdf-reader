//! POC for ADR 0013 (proposed, #90): what the mupdf binding can do for editing and saving,
//! before any of it is wired into the worker. Each `Vec<u8>` stands for the file behind the
//! write-only handle the main process would give the worker (crates/sandbox tests that part).

use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfWriteOptions};

fn corpus(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn write(doc: &MuPdfDocument, options: PdfWriteOptions) -> Vec<u8> {
    let mut out = Vec::new();
    doc.write_to_with_options(&mut out, options).expect("write");
    out
}

fn rotation(pdf: &[u8], page: i32) -> i32 {
    let doc = MuPdfDocument::from_bytes(pdf).expect("reopen");
    doc.load_pdf_page(page)
        .expect("page")
        .rotation()
        .expect("rotation")
}

#[test]
fn an_edit_is_written_to_any_writer_and_opens_again() {
    let doc = MuPdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
    doc.load_pdf_page(0)
        .expect("page")
        .set_rotation(90)
        .expect("rotate");
    let saved = write(&doc, PdfWriteOptions::default());

    assert_eq!(rotation(&saved, 0), 90);
    assert_eq!(rotation(&saved, 1), 0);
    // The worker's own engine opens the result like any other document.
    let engine = pdf_worker::engine::PdfDocument::from_bytes(&saved).expect("engine");
    assert_eq!(engine.page_count().expect("pages"), 10);
}

#[test]
fn an_incremental_save_leaves_a_signed_file_as_it_was() {
    let original = corpus("benign/signed.pdf");
    let doc = MuPdfDocument::from_bytes(&original).expect("open");
    assert!(doc.can_be_saved_incrementally());
    doc.load_pdf_page(0)
        .expect("page")
        .set_rotation(90)
        .expect("rotate");
    let mut options = PdfWriteOptions::default();
    options.set_incremental(true);
    let saved = write(&doc, options);

    // The signed bytes come first, unchanged: the signature's /ByteRange still covers them.
    assert!(saved.len() > original.len());
    assert!(saved[..original.len()] == original[..]);
    assert_eq!(rotation(&saved, 0), 90);
}

#[test]
fn only_a_full_rewrite_drops_what_was_deleted() {
    // The text of page 2 is in the file as it is (the corpus does not compress content).
    let marker = b"Page 2 of 10";
    let original = corpus("benign/multi-page-10.pdf");
    assert!(contains(&original, marker));

    let delete_page_2 = |options: PdfWriteOptions| {
        let mut doc = MuPdfDocument::from_bytes(&original).expect("open");
        doc.delete_page(1).expect("delete");
        write(&doc, options)
    };

    let mut rewrite = PdfWriteOptions::default();
    rewrite.set_garbage(true);
    let rewritten = delete_page_2(rewrite);
    assert!(!contains(&rewritten, marker), "the deleted page is gone");
    let reopened = MuPdfDocument::from_bytes(&rewritten).expect("reopen");
    assert_eq!(reopened.page_count().expect("pages"), 9);

    // An incremental save only appends: the deleted page stays in the file, readable by anyone
    // who looks at the bytes. Hence ADR 0013 rewrites by default.
    let mut incremental = PdfWriteOptions::default();
    incremental.set_incremental(true);
    let appended = delete_page_2(incremental);
    assert!(
        contains(&appended, marker),
        "the deleted page is still in the bytes"
    );
    assert_eq!(
        MuPdfDocument::from_bytes(&appended)
            .expect("reopen")
            .page_count()
            .expect("pages"),
        9
    );
}
