//! Small PDFs for integration tests, built with correct xref tables.

/// Builds a PDF from object bodies (object n = body n-1; object 1 is the catalog).
#[allow(dead_code)]
pub fn build_pdf(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A single Letter page whose content stream is `content`.
#[allow(dead_code)]
pub fn one_page(content: &str) -> Vec<u8> {
    build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ])
}

use std::sync::atomic::{AtomicU32, Ordering};

/// Writes `bytes` to a uniquely named file in the temp directory and returns its path.
#[allow(dead_code)]
pub fn temp_pdf(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("pdf-worker-{}-{id}-{name}.pdf", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

fn is_pdf_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_pdf_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn skip_whitespace_and_comments(pdf: &[u8], pos: &mut usize) {
    while *pos < pdf.len() {
        let b = pdf[*pos];
        if is_pdf_whitespace(b) {
            *pos += 1;
        } else if b == b'%' {
            *pos += 1;
            while *pos < pdf.len() && pdf[*pos] != b'\r' && pdf[*pos] != b'\n' {
                *pos += 1;
            }
        } else {
            break;
        }
    }
}

fn skip_pdf_string(pdf: &[u8], pos: &mut usize) -> Result<(), String> {
    if *pos >= pdf.len() {
        return Err("unexpected end of file while expecting PDF string".to_string());
    }
    match pdf[*pos] {
        b'<' => {
            if pdf.get(*pos + 1) == Some(&b'<') {
                return Err("unexpected '<<' while expecting hex string".to_string());
            }
            *pos += 1;
            while *pos < pdf.len() {
                let b = pdf[*pos];
                *pos += 1;
                if b == b'>' {
                    return Ok(());
                }
                if b.is_ascii_hexdigit() || is_pdf_whitespace(b) {
                    continue;
                }
                return Err(format!("invalid character {b:#04x} in hex string"));
            }
            Err("unterminated hex string".to_string())
        }
        b'(' => {
            *pos += 1;
            let mut depth = 1usize;
            while *pos < pdf.len() {
                let b = pdf[*pos];
                *pos += 1;
                if b == b'\\' {
                    if *pos >= pdf.len() {
                        return Err("unterminated escape in literal string".to_string());
                    }
                    let esc = pdf[*pos];
                    if matches!(esc, b'0'..=b'7') {
                        *pos += 1;
                        if *pos < pdf.len() && matches!(pdf[*pos], b'0'..=b'7') {
                            *pos += 1;
                            if *pos < pdf.len() && matches!(pdf[*pos], b'0'..=b'7') {
                                *pos += 1;
                            }
                        }
                    } else if esc == b'\r' {
                        *pos += 1;
                        if *pos < pdf.len() && pdf[*pos] == b'\n' {
                            *pos += 1;
                        }
                    } else {
                        *pos += 1;
                    }
                } else if b == b'(' {
                    depth += 1;
                } else if b == b')' {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
            }
            Err("unterminated literal string".to_string())
        }
        other => Err(format!(
            "expected string starting with '<' or '(', found byte {other:#04x}"
        )),
    }
}

fn find_last_trailer(pdf: &[u8]) -> Result<usize, String> {
    let mut search_end = pdf.len();
    while let Some(pos) = pdf[..search_end].windows(7).rposition(|w| w == b"trailer") {
        let before_ok =
            pos == 0 || is_pdf_whitespace(pdf[pos - 1]) || is_pdf_delimiter(pdf[pos - 1]);
        let after_ok = pos + 7 == pdf.len()
            || is_pdf_whitespace(pdf[pos + 7])
            || is_pdf_delimiter(pdf[pos + 7]);
        if before_ok && after_ok {
            let mut scan = pos + 7;
            skip_whitespace_and_comments(pdf, &mut scan);
            if pdf[scan..].starts_with(b"<<") {
                return Ok(pos);
            }
        }
        search_end = pos;
    }
    Err("no trailer keyword found".to_string())
}

fn find_trailer_dict_bounds(pdf: &[u8], trailer_pos: usize) -> Result<(usize, usize), String> {
    let mut pos = trailer_pos + 7;
    skip_whitespace_and_comments(pdf, &mut pos);
    if !pdf[pos..].starts_with(b"<<") {
        return Err("trailer dictionary opening '<<' not found".to_string());
    }
    let dict_start = pos + 2;
    pos += 2;
    let mut dict_depth = 1usize;
    while pos < pdf.len() {
        if pdf[pos..].starts_with(b"<<") {
            dict_depth += 1;
            pos += 2;
        } else if pdf[pos..].starts_with(b">>") {
            dict_depth -= 1;
            if dict_depth == 0 {
                let dict_end = pos;
                return Ok((dict_start, dict_end));
            }
            pos += 2;
        } else if pdf[pos] == b'<' || pdf[pos] == b'(' {
            skip_pdf_string(pdf, &mut pos)?;
        } else if pdf[pos] == b'%' {
            pos += 1;
            while pos < pdf.len() && pdf[pos] != b'\r' && pdf[pos] != b'\n' {
                pos += 1;
            }
        } else {
            pos += 1;
        }
    }
    Err("unterminated trailer dictionary".to_string())
}

fn find_trailer_id_span(
    pdf: &[u8],
    dict_start: usize,
    dict_end: usize,
) -> Result<(usize, usize), String> {
    let dict = &pdf[dict_start..dict_end];
    let mut search_pos = 0;
    while let Some(rel_id) = dict[search_pos..].windows(3).position(|w| w == b"/ID") {
        let id_offset = search_pos + rel_id;
        let mut pos = dict_start + id_offset + 3;
        skip_whitespace_and_comments(pdf, &mut pos);
        if pos < dict_end && pdf[pos] == b'[' {
            let id_start = dict_start + id_offset;
            pos += 1; // skip '['
            skip_whitespace_and_comments(pdf, &mut pos);
            skip_pdf_string(pdf, &mut pos)?;
            skip_whitespace_and_comments(pdf, &mut pos);
            skip_pdf_string(pdf, &mut pos)?;
            skip_whitespace_and_comments(pdf, &mut pos);
            if pos >= dict_end || pdf[pos] != b']' {
                return Err("expected ']' closing /ID array in trailer".to_string());
            }
            pos += 1; // skip ']'
            let id_end = pos;
            return Ok((id_start, id_end));
        }
        search_pos = id_offset + 3;
    }
    Err("no /ID entry found in last trailer dictionary".to_string())
}

/// Returns the PDF bytes with the `/ID` entry stripped from the last trailer dictionary.
/// Fails if the trailer or its `/ID` entry is missing or malformed.
#[allow(dead_code)]
pub fn without_trailer_id(pdf: &[u8]) -> Result<Vec<u8>, String> {
    let trailer_pos = find_last_trailer(pdf)?;
    let (dict_start, dict_end) = find_trailer_dict_bounds(pdf, trailer_pos)?;
    let (id_start, id_end) = find_trailer_id_span(pdf, dict_start, dict_end)?;
    let mut out = Vec::with_capacity(pdf.len() - (id_end - id_start));
    out.extend_from_slice(&pdf[..id_start]);
    out.extend_from_slice(&pdf[id_end..]);
    Ok(out)
}
