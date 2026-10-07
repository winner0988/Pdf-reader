//! Verifying the signatures of a document (B2-14, ADR 0014, docs/architecture/signatures.md).
//!
//! The file is the one the document was opened from (or last saved to), not the document as it
//! is edited: a signature covers bytes. For each signature field that has a signature, this
//! checks that the signature's range fits the file and that what the file holds between the two
//! parts of it is exactly the signature, then has Windows verify the signature (`crate::crypt`).
//! Everything read from the file is untrusted, and nothing here is ever fetched.

use std::ops::Range;

use ipc_contract::limits::{MAX_SIGNATURE_CONTENTS_BYTES, MAX_SIGNATURE_TEXT_BYTES};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::{
    Certification, SignatureInfo, SignatureReport, SignatureStatus, UnverifiableReason,
};

use crate::crypt::{self, Verdict};
use crate::engine::{EngineError, PdfDocument, SignatureField};

/// Verifies the signatures of `document`, which was opened from `file`.
pub fn verify(document: &PdfDocument, file: &[u8]) -> Result<SignatureReport, EngineError> {
    let found = document.signature_fields()?;
    Ok(SignatureReport {
        signatures: found
            .fields
            .iter()
            .map(|field| judge(field, file))
            .collect(),
        truncated: found.truncated,
    })
}

/// What was found of one signature.
enum Checked {
    /// It does not hold, or does not fit the file.
    Invalid,
    Unverifiable(UnverifiableReason),
    /// What Windows made of it, and whether the file ends where the signature's range does.
    Windows {
        verdict: Verdict,
        whole_file: bool,
    },
}

fn judge(field: &SignatureField, file: &[u8]) -> SignatureInfo {
    let mut info = SignatureInfo {
        status: SignatureStatus::Invalid,
        signer_trusted: false,
        reason: None,
        field_name: field
            .name
            .as_deref()
            .map(|name| clean_display_text(name, MAX_SIGNATURE_TEXT_BYTES as usize))
            .filter(|name| !name.is_empty()),
        signer: None,
        claimed_time: field.claimed_time.as_deref().and_then(claimed_time),
        certification: field.certification.and_then(certification),
    };
    match check(field, file) {
        Checked::Invalid => {}
        Checked::Unverifiable(reason) => {
            info.status = SignatureStatus::Unverifiable;
            info.reason = Some(reason);
        }
        Checked::Windows {
            verdict,
            whole_file,
        } => match verdict {
            Verdict::Holds { signer, trusted } => {
                info.status = if whole_file {
                    SignatureStatus::Valid
                } else {
                    SignatureStatus::ChangedAfterSigning
                };
                info.signer_trusted = trusted;
                info.signer = Some(clean_display_text(
                    &signer,
                    MAX_SIGNATURE_TEXT_BYTES as usize,
                ))
                .filter(|name| !name.is_empty());
            }
            Verdict::Invalid => {}
            Verdict::UnsupportedAlgorithm => {
                info.status = SignatureStatus::Unverifiable;
                info.reason = Some(UnverifiableReason::UnsupportedAlgorithm);
            }
            Verdict::UnsupportedFormat => {
                info.status = SignatureStatus::Unverifiable;
                info.reason = Some(UnverifiableReason::UnsupportedFormat);
            }
            Verdict::NotAvailable => {
                info.status = SignatureStatus::Unverifiable;
                info.reason = Some(UnverifiableReason::NotAvailable);
            }
        },
    }
    info
}

fn check(field: &SignatureField, file: &[u8]) -> Checked {
    // Only the kinds of signature that are a detached CMS over the file's bytes.
    if !matches!(
        field.sub_filter.as_deref(),
        Some(b"adbe.pkcs7.detached" | b"ETSI.CAdES.detached")
    ) {
        return Checked::Unverifiable(UnverifiableReason::UnsupportedFormat);
    }
    let Some(parts) = field.byte_range.and_then(|range| parts(range, file.len())) else {
        return Checked::Invalid;
    };
    // The file holds the signature as hex text between the two parts: at most twice its size,
    // and the brackets. Larger is refused before anything of it is copied or decoded.
    let gap = &file[parts.first.end..parts.second.start];
    if gap.len() > 2 * MAX_SIGNATURE_CONTENTS_BYTES + 2 {
        return Checked::Unverifiable(UnverifiableReason::TooLarge);
    }
    let Some(in_file) = hex_text(gap) else {
        return Checked::Invalid;
    };
    // The signature as the form's dictionary has it must be what is there: otherwise what the
    // range leaves out is not that signature, whatever it holds.
    if field.contents().as_deref() != Some(in_file.as_slice()) {
        return Checked::Invalid;
    }
    let cms = match der_value(&in_file) {
        Ok(cms) => cms,
        Err(DerProblem::Indefinite) => {
            return Checked::Unverifiable(UnverifiableReason::UnsupportedFormat);
        }
        Err(DerProblem::Malformed) => return Checked::Invalid,
    };
    let verdict = crypt::verify_detached(
        [&file[parts.first.clone()], &file[parts.second.clone()]],
        cms,
    );
    Checked::Windows {
        verdict,
        whole_file: is_blank(&file[parts.second.end..]),
    }
}

/// The two parts of the file a signature covers, and so the hex text between them.
struct Parts {
    first: Range<usize>,
    second: Range<usize>,
}

/// The parts `/ByteRange` says, if they fit a file of `len` bytes the way a signature's do: the
/// first starts the file, then comes room for the signature's hex string (at least `<>`), then the
/// second, which ends inside the file.
fn parts(range: [i32; 4], len: usize) -> Option<Parts> {
    let [Some(start), Some(first), Some(offset), Some(rest)] =
        range.map(|number| usize::try_from(number).ok())
    else {
        return None;
    };
    let end = offset.checked_add(rest)?;
    (start == 0 && first > 0 && offset >= first.checked_add(2)? && rest > 0 && end <= len)
        .then_some(Parts {
            first: 0..first,
            second: offset..end,
        })
}

/// The bytes of the hex string `text` (`<` and `>` around the digits, white space skipped), or
/// `None` if it is anything else. A last digit alone counts as followed by 0 (PDF 32000-1,
/// 7.3.4.3).
fn hex_text(text: &[u8]) -> Option<Vec<u8>> {
    let digits = text.strip_prefix(b"<")?.strip_suffix(b">")?;
    let mut bytes = Vec::with_capacity(digits.len() / 2);
    let mut high = None;
    for &byte in digits {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            0 | b'\t' | b'\n' | 0x0c | b'\r' | b' ' => continue,
            _ => return None,
        };
        match high.take() {
            Some(high) => bytes.push(high << 4 | digit),
            None => high = Some(digit),
        }
    }
    if let Some(high) = high {
        bytes.push(high << 4);
    }
    Some(bytes)
}

enum DerProblem {
    /// The length is not given (BER's indefinite form).
    Indefinite,
    Malformed,
}

/// The DER value (a SEQUENCE: the CMS `ContentInfo`) at the start of `bytes`; what follows it is
/// the padding the signature's room in the file was filled up with, and is left out.
fn der_value(bytes: &[u8]) -> Result<&[u8], DerProblem> {
    let [0x30, length, rest @ ..] = bytes else {
        return Err(DerProblem::Malformed);
    };
    let (header, length) = match *length {
        0x80 => return Err(DerProblem::Indefinite),
        short @ 0..0x80 => (2, usize::from(short)),
        long => {
            let count = usize::from(long & 0x7f);
            // Four length bytes are more than a signature's room can hold.
            let digits = rest.get(..count).filter(|_| (1..=4).contains(&count));
            let digits = digits.ok_or(DerProblem::Malformed)?;
            let length = digits
                .iter()
                .fold(0usize, |sum, byte| (sum << 8) | usize::from(*byte));
            (2 + count, length)
        }
    };
    let end = header.checked_add(length).ok_or(DerProblem::Malformed)?;
    bytes.get(..end).ok_or(DerProblem::Malformed)
}

/// Whether `tail` is only white space: after the last part of a signature that is no change (a
/// line break after `%%EOF` is common), anything else is.
fn is_blank(tail: &[u8]) -> bool {
    tail.iter()
        .all(|byte| matches!(byte, 0 | b'\t' | b'\n' | 0x0c | b'\r' | b' '))
}

fn certification(permission: i32) -> Option<Certification> {
    match permission {
        1 => Some(Certification::NoChanges),
        2 => Some(Certification::FillForms),
        3 => Some(Certification::FillFormsAndAnnotate),
        _ => None,
    }
}

/// A PDF date (`D:YYYYMMDDHHmmSSOHH'mm'`, whose end is optional) as text for display, if it has
/// at least the minute: "2026-09-24 12:00:00 UTC+08:00". `None` for anything else.
fn claimed_time(date: &str) -> Option<String> {
    let date = date.trim();
    let date = date.strip_prefix("D:").unwrap_or(date).as_bytes();
    let count = date.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if !(12..=14).contains(&count) || count % 2 != 0 {
        return None;
    }
    let number = |from: usize, len: usize| -> Option<u32> {
        let digits = date.get(from..from + len)?;
        digits.iter().try_fold(0, |sum, digit| {
            Some(sum * 10 + u32::from(digit.checked_sub(b'0')?))
        })
    };
    let (year, month, day) = (number(0, 4)?, number(4, 2)?, number(6, 2)?);
    let (hour, minute) = (number(8, 2)?, number(10, 2)?);
    let second = if count == 14 { number(12, 2)? } else { 0 };
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let zone = match &date[count..] {
        [] => String::new(),
        [b'Z', ..] => " UTC".to_owned(),
        [sign @ (b'+' | b'-'), rest @ ..] => {
            let rest = std::str::from_utf8(rest).ok()?;
            let digits: String = rest.chars().filter(char::is_ascii_digit).collect();
            let (hours, minutes) = match digits.len() {
                2 => (digits.as_str(), "00"),
                4 => (&digits[..2], &digits[2..]),
                _ => return None,
            };
            // The PDF form is HH'mm' (or HH'mm); nothing else is a zone.
            if rest.chars().any(|c| !c.is_ascii_digit() && c != '\'')
                || hours.parse::<u32>().ok()? > 23
                || minutes.parse::<u32>().ok()? > 59
            {
                return None;
            }
            format!(" UTC{}{hours}:{minutes}", char::from(*sign))
        }
        _ => return None,
    };
    Some(format!(
        "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}{zone}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_must_fit_the_file_the_way_a_signatures_does() {
        let fits = |range| parts(range, 1_000).map(|parts| (parts.first, parts.second));
        assert_eq!(fits([0, 100, 300, 700]), Some((0..100, 300..1000)));
        // The room for the signature has at least its brackets.
        assert!(fits([0, 100, 101, 899]).is_none());
        assert!(fits([0, 100, 102, 898]).is_some());
        // Not from the start, nothing in a part, negative, past the end, backwards, absurd.
        for range in [
            [1, 100, 300, 600],
            [0, 0, 300, 700],
            [0, 100, 300, 0],
            [0, -1, 300, 700],
            [0, 100, -300, 700],
            [0, 100, 300, 701],
            [0, 400, 300, 700],
            [i32::MAX, 100, 300, 700],
            [0, 100, i32::MAX, i32::MAX],
        ] {
            assert!(fits(range).is_none(), "{range:?}");
        }
    }

    #[test]
    fn the_hex_text_between_the_parts_is_a_string_and_nothing_else() {
        assert_eq!(hex_text(b"<30820a0B>"), Some(vec![0x30, 0x82, 0x0a, 0x0b]));
        assert_eq!(
            hex_text(b"<30 82\r\n0a\t0B 0>"),
            Some(vec![0x30, 0x82, 0x0a, 0x0b, 0x00])
        );
        assert_eq!(hex_text(b"<>"), Some(vec![]));
        assert_eq!(hex_text(b"<abc>"), Some(vec![0xab, 0xc0]));
        for bad in [
            &b"30820a0b"[..],
            b"<3082",
            b"3082>",
            b"<30 /Other 82>",
            b"<30>>",
            b"<3g>",
            b"(3082)",
            b"<30><31>",
            b"",
        ] {
            assert_eq!(hex_text(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_signature_is_the_der_value_at_the_start_of_its_room() {
        let der = |bytes: &[u8]| {
            der_value(bytes)
                .map(<[u8]>::to_vec)
                .map_err(|problem| matches!(problem, DerProblem::Indefinite))
        };
        assert_eq!(
            der(&[0x30, 0x02, 1, 2, 0, 0, 0]),
            Ok(vec![0x30, 0x02, 1, 2])
        );
        assert_eq!(
            der(&[0x30, 0x81, 0x03, 1, 2, 3, 0]),
            Ok(vec![0x30, 0x81, 3, 1, 2, 3])
        );
        assert_eq!(
            der(&[0x30, 0x82, 0x00, 0x01, 9]),
            Ok(vec![0x30, 0x82, 0, 1, 9])
        );
        // BER's indefinite length is told apart from damage.
        assert_eq!(der(&[0x30, 0x80, 1, 0, 0]), Err(true));
        for bad in [
            &[][..],
            &[0x30],
            &[0x31, 0x00],
            &[0x30, 0x03, 1, 2],
            &[0x30, 0x81],
            &[0x30, 0x85, 0, 0, 0, 0, 1, 0],
            &[0x30, 0x84, 0xff, 0xff, 0xff, 0xff, 0],
        ] {
            assert_eq!(der(bad), Err(false), "{bad:?}");
        }
    }

    #[test]
    fn only_white_space_after_the_last_part_is_no_change() {
        assert!(is_blank(b""));
        assert!(is_blank(b"\r\n \t\0"));
        assert!(!is_blank(b"\nxref"));
        assert!(!is_blank(b"%%EOF"));
    }

    #[test]
    fn a_certifying_signature_says_what_may_change() {
        assert_eq!(certification(1), Some(Certification::NoChanges));
        assert_eq!(certification(2), Some(Certification::FillForms));
        assert_eq!(certification(3), Some(Certification::FillFormsAndAnnotate));
        assert_eq!(certification(0), None);
        assert_eq!(certification(4), None);
    }

    #[test]
    fn the_time_a_signer_claims_is_shown_as_text_or_not_at_all() {
        let shown = |date: &str| claimed_time(date).unwrap_or_default();
        assert_eq!(
            shown("D:20260924120000+08'00'"),
            "2026-09-24 12:00:00 UTC+08:00"
        );
        assert_eq!(
            shown("D:20260924120000-05'30"),
            "2026-09-24 12:00:00 UTC-05:30"
        );
        assert_eq!(
            shown("D:20260924120000+08"),
            "2026-09-24 12:00:00 UTC+08:00"
        );
        assert_eq!(shown("D:20260924120000Z"), "2026-09-24 12:00:00 UTC");
        assert_eq!(shown("D:20260924120000Z00'00'"), "2026-09-24 12:00:00 UTC");
        // No zone is no guess at one; no seconds are none.
        assert_eq!(shown("D:20260924120000"), "2026-09-24 12:00:00");
        assert_eq!(shown("D:202609241200"), "2026-09-24 12:00:00");
        assert_eq!(
            shown(" 20260924120000+08'00' "),
            "2026-09-24 12:00:00 UTC+08:00"
        );
        for bad in [
            "",
            "yesterday",
            "D:2026",
            "D:20260924",
            "D:2026092412000",
            "D:20261324120000",
            "D:20260932120000",
            "D:20260924240000",
            "D:20260924126000",
            "D:20260924120060",
            "D:20260924120000+24'00'",
            "D:20260924120000+08'60'",
            "D:20260924120000+8'00'",
            "D:20260924120000 +08'00'",
            "D:20260924120000x",
            "D:2026-09-24 12:00",
        ] {
            assert_eq!(claimed_time(bad), None, "{bad}");
        }
    }
}
