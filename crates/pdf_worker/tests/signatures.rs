//! The signatures of a document verified by the real worker in the real sandbox (B2-14, ADR 0014):
//! Windows' own cryptography, run next to the PDF parser, with nothing asked of the network.
//! The corpus has two signed files, both made with a self-signed test certificate: their
//! signatures hold, and nobody vouches for the signer.
#![cfg(windows)]

use std::path::Path;

use ipc_contract::limits::MAX_SIGNATURES;
use ipc_contract::types::{
    Certification, SignatureInfo, SignatureReport, SignatureStatus, UnverifiableReason,
};
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfWriteOptions};
use worker_host::{HostConfig, WorkerHost};

// Shared with the other test crates, which use the helpers this one does not.
#[allow(dead_code)]
mod common;

/// The corpus's self-signed test certificate (tests/corpus/generate.py, `SIGNER_NAME`).
const SIGNER: &str = "PDF Reader test corpus signer (NOT TRUSTED)";

fn corpus(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// What the worker says of the signatures of `pdf`, which it opens as a file named `name`.
fn report_of(pdf: &[u8], name: &str) -> SignatureReport {
    let path = common::temp_pdf(name, pdf);
    let mut host = WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    );
    let (doc, opened) = host.open(&path).expect("open");
    assert!(
        matches!(opened, WorkerResponse::Opened { .. }),
        "{opened:?}"
    );
    let answer = host
        .request(|request| WorkerRequest::VerifySignatures { request, doc })
        .expect("verify");
    std::fs::remove_file(path).ok();
    match answer {
        WorkerResponse::Signatures { report, .. } => report,
        other => panic!("{other:?}"),
    }
}

fn only(report: &SignatureReport) -> &SignatureInfo {
    assert!(!report.truncated);
    let [signature] = &report.signatures[..] else {
        panic!("one signature: {report:?}");
    };
    signature
}

fn position(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or_else(|| panic!("{}", String::from_utf8_lossy(needle)))
}

#[test]
fn the_signatures_of_the_corpus_hold_but_nobody_vouches_for_the_signer() {
    for (name, certification) in [
        ("benign/signed.pdf", None),
        (
            "benign/signed-docmdp-p1.pdf",
            Some(Certification::NoChanges),
        ),
    ] {
        let report = report_of(&corpus(name), "signed");
        assert_eq!(
            only(&report),
            &SignatureInfo {
                status: SignatureStatus::Valid,
                signer_trusted: false,
                reason: None,
                field_name: Some("Signature1".to_owned()),
                signer: Some(SIGNER.to_owned()),
                claimed_time: Some("2026-01-01 00:00:00 UTC".to_owned()),
                certification,
            },
            "{name}"
        );
    }
}

#[test]
fn a_change_inside_what_was_signed_makes_the_signature_invalid() {
    let mut pdf = corpus("benign/signed.pdf");
    // One letter of the page's text, inside the signed range.
    let at = position(&pdf, b"Signed sample");
    pdf[at] = b's';
    let report = report_of(&pdf, "signed-changed");
    let signature = only(&report);
    assert_eq!(signature.status, SignatureStatus::Invalid);
    // A signature that does not hold names nobody.
    assert_eq!(
        (signature.signer.as_deref(), signature.signer_trusted),
        (None, false)
    );
}

#[test]
fn a_change_to_the_signature_itself_makes_it_invalid() {
    let mut pdf = corpus("benign/signed.pdf");
    // A digit inside the signature value, which ends the CMS: its hex is not covered by anything.
    let start = position(&pdf, b"/Contents <") + b"/Contents <".len();
    let length =
        |at: usize| usize::from_str_radix(std::str::from_utf8(&pdf[at..at + 4]).unwrap(), 16);
    // 30 82 LL LL: a SEQUENCE with a two-byte length.
    assert_eq!(&pdf[start..start + 4], b"3082");
    let der = 4 + length(start + 4).unwrap();
    let at = start + 2 * (der - 20);
    pdf[at] = if pdf[at] == b'0' { b'1' } else { b'0' };
    let report = report_of(&pdf, "signed-damaged");
    assert_eq!(only(&report).status, SignatureStatus::Invalid);
}

#[test]
fn a_change_after_signing_is_told_but_the_signature_still_holds_for_what_it_covers() {
    let original = corpus("benign/signed.pdf");
    let doc = MuPdfDocument::from_bytes(&original).expect("open");
    doc.load_pdf_page(0)
        .expect("page")
        .set_rotation(90)
        .expect("rotate");
    let mut options = PdfWriteOptions::default();
    options.set_incremental(true);
    let mut updated = Vec::new();
    doc.write_to_with_options(&mut updated, options)
        .expect("write");
    assert!(updated.len() > original.len());
    assert!(updated.starts_with(&original));

    let report = report_of(&updated, "signed-updated");
    let signature = only(&report);
    assert_eq!(signature.status, SignatureStatus::ChangedAfterSigning);
    assert_eq!(signature.signer.as_deref(), Some(SIGNER));
    assert!(!signature.signer_trusted);
}

#[test]
fn white_space_after_the_end_of_the_file_is_no_change() {
    let mut pdf = corpus("benign/signed.pdf");
    pdf.extend_from_slice(b"\r\n\r\n");
    let report = report_of(&pdf, "signed-newline");
    assert_eq!(only(&report).status, SignatureStatus::Valid);
}

#[test]
fn a_kind_of_signature_that_is_not_checked_is_said_to_be_unverifiable() {
    let mut pdf = corpus("benign/signed.pdf");
    // Another name of the same length: the byte range stays what it was.
    let kind = b"adbe.pkcs7.detached";
    let at = position(&pdf, kind);
    pdf[at..at + kind.len()].copy_from_slice(b"adbe.pkcs7.sha1xxxx");
    let report = report_of(&pdf, "signed-other-kind");
    let signature = only(&report);
    assert_eq!(signature.status, SignatureStatus::Unverifiable);
    assert_eq!(
        signature.reason,
        Some(UnverifiableReason::UnsupportedFormat)
    );
    assert_eq!(
        (signature.signer.as_deref(), signature.signer_trusted),
        (None, false)
    );
}

#[test]
fn a_document_without_signatures_has_none_to_report() {
    let report = report_of(&corpus("benign/mixed-page-sizes.pdf"), "unsigned");
    assert_eq!(report, SignatureReport::default());
    // A form with fields of other kinds has none either.
    let report = report_of(&corpus("benign/form-fields.pdf"), "form");
    assert_eq!(report, SignatureReport::default());
}

/// A PDF whose form has `fields` as its top-level fields (object 5 on) and the given extra objects
/// after them.
fn form_pdf(fields: &[String], extra: &[String]) -> Vec<u8> {
    form_pdf_with("", fields, extra)
}

/// The same, whose page has `content` as its content stream.
fn form_pdf_with(content: &str, fields: &[String], extra: &[String]) -> Vec<u8> {
    let refs: Vec<String> = (0..fields.len())
        .map(|i| format!("{} 0 R", 5 + i))
        .collect();
    let mut objects = vec![
        format!(
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [{}] >> >>",
            refs.join(" ")
        ),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    objects.extend(fields.iter().cloned());
    objects.extend(extra.iter().cloned());
    common::build_pdf(&objects)
}

/// A signature field that claims a signature it does not have.
fn hollow_field(name: usize) -> String {
    format!(
        "<< /FT /Sig /T (S{name}) /V << /Type /Sig /SubFilter /adbe.pkcs7.detached \
         /ByteRange [0 1 2 3] /Contents <00> >> >>"
    )
}

#[test]
fn more_signature_fields_than_are_looked_at_are_cut_and_said_so() {
    let fields: Vec<String> = (0..MAX_SIGNATURES as usize + 1).map(hollow_field).collect();
    let report = report_of(&form_pdf(&fields, &[]), "many-signatures");
    assert!(report.truncated);
    assert_eq!(report.signatures.len(), MAX_SIGNATURES as usize);
    // A range that does not fit the file is no signature that holds.
    assert!(report.signatures.iter().all(
        |signature| signature.status == SignatureStatus::Invalid && signature.signer.is_none()
    ));
    assert_eq!(report.signatures[0].field_name.as_deref(), Some("S0"));
    assert_eq!(report.signatures[99].field_name.as_deref(), Some("S99"));
}

#[test]
fn fields_that_are_their_own_kids_are_followed_once() {
    // Object 5 has itself and object 6 as kids; object 6 has object 5.
    let fields = [
        "<< /FT /Sig /T (loop) /Kids [5 0 R 6 0 R] /V << /Type /Sig /SubFilter /adbe.pkcs7.detached \
         /ByteRange [0 1 2 3] /Contents <00> >> >>"
            .to_owned(),
        "<< /T (inner) /Kids [5 0 R] >>".to_owned(),
    ];
    let report = report_of(&form_pdf(&fields, &[]), "loop");
    assert_eq!(report.signatures.len(), 1);
    assert_eq!(report.signatures[0].field_name.as_deref(), Some("loop"));
}

#[test]
fn a_signature_that_several_fields_share_is_one() {
    // Both fields point at the signature in object 7.
    let fields = [
        "<< /FT /Sig /T (first) /V 7 0 R >>".to_owned(),
        "<< /FT /Sig /T (second) /V 7 0 R >>".to_owned(),
    ];
    let extra = [
        "<< /Type /Sig /SubFilter /adbe.pkcs7.detached /ByteRange [0 1 2 3] /Contents <00> >>"
            .to_owned(),
    ];
    let report = report_of(&form_pdf(&fields, &extra), "shared");
    assert_eq!(report.signatures.len(), 1);
    assert_eq!(report.signatures[0].field_name.as_deref(), Some("first"));
}

#[test]
fn the_type_of_a_field_is_the_one_of_the_fields_above_it() {
    // The kid has no type of its own: it is a signature field because its parent is one. The text
    // field's kid with a value is not.
    let fields = [
        "<< /FT /Sig /T (group) /Kids [7 0 R] >>".to_owned(),
        "<< /FT /Tx /T (text) /Kids [8 0 R] >>".to_owned(),
    ];
    let extra = [
        "<< /T (kid) /V << /Type /Sig /SubFilter /adbe.pkcs7.detached /ByteRange [0 1 2 3] \
         /Contents <00> >> >>"
            .to_owned(),
        "<< /T (not-signature) /V << /Type /Sig /SubFilter /adbe.pkcs7.detached \
         /ByteRange [0 1 2 3] /Contents <00> >> >>"
            .to_owned(),
    ];
    let report = report_of(&form_pdf(&fields, &extra), "inherited");
    assert_eq!(report.signatures.len(), 1);
    assert_eq!(report.signatures[0].field_name.as_deref(), Some("kid"));
}

/// `pdf` with the three numbers of the placeholder `/ByteRange [0 0000000000 0000000000
/// 0000000000]` set to leave out `gap`, whose bytes are somewhere in it.
fn with_range_around(mut pdf: Vec<u8>, gap: std::ops::Range<usize>) -> Vec<u8> {
    let head = b"/ByteRange [0 ";
    let at = position(&pdf, head) + head.len();
    let numbers = format!(
        "{:010} {:010} {:010}",
        gap.start,
        gap.end,
        pdf.len() - gap.end
    );
    pdf[at..at + numbers.len()].copy_from_slice(numbers.as_bytes());
    pdf
}

const PLACEHOLDER_RANGE: &str = "[0 0000000000 0000000000 0000000000]";

#[test]
fn a_range_that_leaves_out_something_else_than_the_signature_is_invalid() {
    // The signature dictionary's /Contents is a string of its own (object 7); the byte range
    // leaves out a hex string in the page's content instead.
    let fields = ["<< /FT /Sig /T (swap) /V 6 0 R >>".to_owned()];
    let extra = [
        format!(
            "<< /Type /Sig /SubFilter /adbe.pkcs7.detached /ByteRange {PLACEHOLDER_RANGE} /Contents 7 0 R >>"
        ),
        "<00>".to_owned(),
    ];
    let pdf = form_pdf_with("<DEADBEEF>", &fields, &extra);
    let at = position(&pdf, b"<DEADBEEF>");
    let pdf = with_range_around(pdf, at..at + b"<DEADBEEF>".len());
    let report = report_of(&pdf, "swapped-signature");
    assert_eq!(only(&report).status, SignatureStatus::Invalid);
}

#[test]
fn a_signature_larger_than_the_app_looks_at_is_said_to_be_too_large() {
    // The range leaves out a hex string of more than twice the largest signature.
    let digits = 2 * ipc_contract::limits::MAX_SIGNATURE_CONTENTS_BYTES + 2;
    let hex = format!("<{}>", "0".repeat(digits));
    let fields = ["<< /FT /Sig /T (big) /V 6 0 R >>".to_owned()];
    let extra = [format!(
        "<< /Type /Sig /SubFilter /adbe.pkcs7.detached /ByteRange {PLACEHOLDER_RANGE} /Contents <00> >>"
    )];
    let pdf = form_pdf_with(&hex, &fields, &extra);
    let at = position(&pdf, hex.as_bytes());
    let pdf = with_range_around(pdf, at..at + hex.len());
    let report = report_of(&pdf, "big-signature");
    let signature = only(&report);
    assert_eq!(signature.status, SignatureStatus::Unverifiable);
    assert_eq!(signature.reason, Some(UnverifiableReason::TooLarge));
}

#[test]
fn a_signature_the_form_no_longer_has_is_not_the_one_the_range_leaves_out() {
    // After signing, an update gives the signature dictionary another /Contents: the signature
    // that the range leaves out is no longer the form's, though it still holds for the bytes it
    // covers.
    let original = corpus("benign/signed.pdf");
    let text = String::from_utf8_lossy(&original).into_owned();
    let start = text.find("6 0 obj").unwrap();
    let end = start + text[start..].find("endobj").unwrap() + "endobj".len();
    let object = &text[start..end];
    let hex_start = object.find("/Contents <").unwrap() + "/Contents ".len();
    let hex_end = hex_start + object[hex_start..].find('>').unwrap() + 1;
    let replaced = format!("{}<00>{}", &object[..hex_start], &object[hex_end..]);
    let previous: usize = text[text.rfind("startxref").unwrap() + "startxref".len()..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();

    let mut updated = original.clone();
    let object_at = updated.len();
    updated.extend_from_slice(replaced.as_bytes());
    updated.push(b'\n');
    let xref_at = updated.len();
    updated.extend_from_slice(
        format!(
            "xref\n6 1\n{object_at:010} 00000 n \ntrailer\n<< /Size 8 /Root 1 0 R /Prev {previous} >>\nstartxref\n{xref_at}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let report = report_of(&updated, "signed-swapped");
    assert_eq!(only(&report).status, SignatureStatus::Invalid);
}
