use ipc_contract::worker::{
    ExportedAnnotation, ExportedAnnotationKind, ExportedAnnotations, ExportedLineEnding,
};

/// Formats a float coordinate/value up to 4 decimals, trimming trailing zeros and dot.
/// Never outputs scientific notation or "-0".
pub fn format_coord(v: f32) -> String {
    if v.abs() < 1e-6 {
        return "0".to_string();
    }
    let s = format!("{:.4}", v);
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" || trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Drops invalid XML 1.0 characters and normalizes CRLF/CR to LF.
pub fn clean_xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{0000}'..='\u{0008}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000E}'..='\u{001F}'
            | '\u{FFFE}'
            | '\u{FFFF}' => {
                // Drop invalid XML 1.0 characters
            }
            valid => out.push(valid),
        }
    }
    out
}

/// Escapes XML attribute value: &, <, >, ", '
pub fn escape_attribute(s: &str) -> String {
    let cleaned = clean_xml_text(s);
    let mut out = String::with_capacity(cleaned.len());
    for c in cleaned.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Escapes XML element text content: &, <, >
pub fn escape_text_content(s: &str) -> String {
    let cleaned = clean_xml_text(s);
    let mut out = String::with_capacity(cleaned.len());
    for c in cleaned.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

fn format_hex_color(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

fn line_ending_name(ending: ExportedLineEnding) -> &'static str {
    match ending {
        ExportedLineEnding::None => "None",
        ExportedLineEnding::OpenArrow => "OpenArrow",
        ExportedLineEnding::ClosedArrow => "ClosedArrow",
    }
}

/// Generates an ISO 19444-1 compliant XFDF XML string from exported annotations (B5-09).
///
/// Privacy guarantees:
/// - No user paths (only the base file name in `<f href="...">`)
/// - No author (`title`), dates (`date`, `creationdate`), subject, or `<ids>` elements.
pub fn file(annotations: &ExportedAnnotations, file_name: &str) -> String {
    let base_name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let href = escape_attribute(base_name);

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n");
    out.push_str("<annots>\n");

    for item in &annotations.items {
        format_annotation(&mut out, item);
    }

    out.push_str("</annots>\n");
    out.push_str(&format!("<f href=\"{href}\"/>\n"));
    out.push_str("</xfdf>\n");

    out
}

fn format_annotation(out: &mut String, item: &ExportedAnnotation) {
    let tag = match item.kind {
        ExportedAnnotationKind::Highlight => "highlight",
        ExportedAnnotationKind::Underline => "underline",
        ExportedAnnotationKind::StrikeOut => "strikeout",
        ExportedAnnotationKind::Squiggly => "squiggly",
        ExportedAnnotationKind::Note => "text",
        ExportedAnnotationKind::Ink => "ink",
        ExportedAnnotationKind::Stamp => "stamp",
        ExportedAnnotationKind::TextBox => "freetext",
        ExportedAnnotationKind::Rectangle => "square",
        ExportedAnnotationKind::Ellipse => "circle",
        ExportedAnnotationKind::Line => "line",
    };

    let rect_str = format!(
        "{},{},{},{}",
        format_coord(item.rect[0]),
        format_coord(item.rect[1]),
        format_coord(item.rect[2]),
        format_coord(item.rect[3])
    );

    let mut attrs = Vec::new();
    attrs.push(format!("page=\"{}\"", item.page));
    attrs.push(format!("rect=\"{rect_str}\""));

    if item.print && item.hidden {
        attrs.push("flags=\"print,hidden\"".to_string());
    } else if item.print {
        attrs.push("flags=\"print\"".to_string());
    } else if item.hidden {
        attrs.push("flags=\"hidden\"".to_string());
    }

    attrs.push(format!("name=\"{}\"", escape_attribute(&item.name)));

    if let Some(color) = item.color {
        attrs.push(format!("color=\"{}\"", format_hex_color(color)));
    }

    if let Some(interior) = item.interior.filter(|_| {
        matches!(
            item.kind,
            ExportedAnnotationKind::Rectangle | ExportedAnnotationKind::Ellipse
        )
    }) {
        attrs.push(format!("interior-color=\"{}\"", format_hex_color(interior)));
    }

    if let Some(opacity) = item.opacity.filter(|&o| o < 0.9999) {
        attrs.push(format!("opacity=\"{}\"", format_coord(opacity)));
    }

    if let Some(width) = item.width.filter(|&w| (w - 1.0).abs() > 1e-4) {
        attrs.push(format!("width=\"{}\"", format_coord(width)));
    }

    if let Some(ref irt) = item.in_reply_to {
        attrs.push(format!("inreplyto=\"{}\"", escape_attribute(irt)));
    }

    // Kind-specific attributes
    match item.kind {
        ExportedAnnotationKind::Highlight
        | ExportedAnnotationKind::Underline
        | ExportedAnnotationKind::StrikeOut
        | ExportedAnnotationKind::Squiggly => {
            if !item.quads.is_empty() {
                let coords: Vec<String> = item
                    .quads
                    .iter()
                    .flat_map(|q| q.iter().map(|&v| format_coord(v)))
                    .collect();
                attrs.push(format!("coords=\"{}\"", coords.join(",")));
            }
        }
        ExportedAnnotationKind::Note => {
            if let Some(ref icon) = item.icon {
                attrs.push(format!("icon=\"{}\"", escape_attribute(icon)));
            }
        }
        ExportedAnnotationKind::Stamp => {
            if let Some(ref icon) = item.icon {
                attrs.push(format!("icon=\"{}\"", escape_attribute(icon)));
            }
        }
        ExportedAnnotationKind::Line => {
            if let Some(coords) = item.line {
                attrs.push(format!(
                    "start=\"{},{}\"",
                    format_coord(coords[0]),
                    format_coord(coords[1])
                ));
                attrs.push(format!(
                    "end=\"{},{}\"",
                    format_coord(coords[2]),
                    format_coord(coords[3])
                ));
            }
            if let Some([head, tail]) = item.line_endings {
                attrs.push(format!("head=\"{}\"", line_ending_name(head)));
                attrs.push(format!("tail=\"{}\"", line_ending_name(tail)));
            }
        }
        ExportedAnnotationKind::TextBox => {
            if let Some(style) = item.text_style {
                let just = match style.justification {
                    1 => "centered",
                    2 => "right",
                    _ => "left",
                };
                attrs.push(format!("justification=\"{just}\""));
            }
        }
        ExportedAnnotationKind::Ink
        | ExportedAnnotationKind::Rectangle
        | ExportedAnnotationKind::Ellipse => {}
    }

    // Child elements
    let mut children = Vec::new();

    if let (ExportedAnnotationKind::TextBox, Some(style)) = (item.kind, item.text_style) {
        let r_f = format_coord(f32::from(style.color[0]) / 255.0);
        let g_f = format_coord(f32::from(style.color[1]) / 255.0);
        let b_f = format_coord(f32::from(style.color[2]) / 255.0);
        let size_f = format_coord(style.size);
        let hex = format_hex_color(style.color);
        children.push(format!(
            "<defaultappearance>/Helvetica {size_f} Tf {r_f} {g_f} {b_f} rg</defaultappearance>"
        ));
        children.push(format!(
            "<defaultstyle>font: {size_f}pt Helvetica; color: {hex}</defaultstyle>"
        ));
    }

    if item.kind == ExportedAnnotationKind::Ink && !item.ink.is_empty() {
        let mut inklist = String::from("<inklist>\n");
        for stroke in &item.ink {
            let gesture: Vec<String> = stroke
                .iter()
                .map(|p| format!("{},{}", format_coord(p[0]), format_coord(p[1])))
                .collect();
            inklist.push_str(&format!("<gesture>{}</gesture>\n", gesture.join(";")));
        }
        inklist.push_str("</inklist>");
        children.push(inklist);
    }

    if let Some(text) = item.contents.as_deref().filter(|s| !s.is_empty()) {
        children.push(format!(
            "<contents>{}</contents>",
            escape_text_content(text)
        ));
    }

    let attr_str = attrs.join(" ");
    if children.is_empty() {
        out.push_str(&format!("<{tag} {attr_str}/>\n"));
    } else {
        out.push_str(&format!("<{tag} {attr_str}>\n"));
        for child in children {
            out.push_str(&format!("{child}\n"));
        }
        out.push_str(&format!("</{tag}>\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc_contract::worker::{
        ExportedAnnotation, ExportedAnnotationKind, ExportedAnnotations, ExportedLineEnding,
    };

    #[test]
    fn format_coord_limits_decimals_and_avoids_negative_zero() {
        assert_eq!(format_coord(0.0), "0");
        assert_eq!(format_coord(-0.0), "0");
        assert_eq!(format_coord(0.00001), "0");
        assert_eq!(format_coord(12.34567), "12.3457");
        assert_eq!(format_coord(100.0), "100");
        assert_eq!(format_coord(0.5000), "0.5");
        assert_eq!(format_coord(-50.25), "-50.25");
    }

    #[test]
    fn clean_xml_text_drops_invalid_xml_and_normalizes_line_endings() {
        let raw = "Hello\x00World\r\nLine 2\rLine 3\x08End\u{FFFF}";
        let cleaned = clean_xml_text(raw);
        assert_eq!(cleaned, "HelloWorld\nLine 2\nLine 3End");
    }

    #[test]
    fn escape_attribute_escapes_all_required_chars() {
        let text = "<tag attr='value' & \"quoted\">";
        assert_eq!(
            escape_attribute(text),
            "&lt;tag attr=&apos;value&apos; &amp; &quot;quoted&quot;&gt;"
        );
    }

    #[test]
    fn escape_text_content_escapes_ampersand_and_brackets() {
        let text = "Rock & Roll <music> 'quotes' \"here\"";
        assert_eq!(
            escape_text_content(text),
            "Rock &amp; Roll &lt;music&gt; 'quotes' \"here\""
        );
    }

    #[test]
    fn file_href_strips_paths_and_keeps_only_base_filename() {
        let annotations = ExportedAnnotations {
            items: vec![],
            skipped: 0,
            truncated: false,
        };

        let xfdf_win = file(&annotations, r"C:\Users\Alice\Documents\report.pdf");
        assert!(xfdf_win.contains(r#"<f href="report.pdf"/>"#));

        let xfdf_unix = file(&annotations, "/home/alice/docs/test.pdf");
        assert!(xfdf_unix.contains(r#"<f href="test.pdf"/>"#));
    }

    #[test]
    fn file_generates_standard_xfdf_with_exact_strings_and_no_private_data() {
        let annotations = ExportedAnnotations {
            items: vec![
                ExportedAnnotation {
                    kind: ExportedAnnotationKind::Highlight,
                    page: 0,
                    rect: [72.0, 700.0, 200.0, 720.0],
                    name: "pdf-reader-1-1".to_string(),
                    in_reply_to: None,
                    color: Some([255, 255, 0]),
                    interior: None,
                    opacity: Some(0.8),
                    width: None,
                    print: true,
                    hidden: false,
                    icon: None,
                    contents: Some("Highlight text".to_string()),
                    quads: vec![[72.0, 720.0, 200.0, 720.0, 72.0, 700.0, 200.0, 700.0]],
                    ink: vec![],
                    line: None,
                    line_endings: None,
                    text_style: None,
                },
                ExportedAnnotation {
                    kind: ExportedAnnotationKind::Note,
                    page: 0,
                    rect: [50.0, 650.0, 70.0, 670.0],
                    name: "pdf-reader-1-2".to_string(),
                    in_reply_to: Some("pdf-reader-1-1".to_string()),
                    color: Some([0, 128, 255]),
                    interior: None,
                    opacity: None,
                    width: None,
                    print: true,
                    hidden: false,
                    icon: Some("Comment".to_string()),
                    contents: Some("Reply note".to_string()),
                    quads: vec![],
                    ink: vec![],
                    line: None,
                    line_endings: None,
                    text_style: None,
                },
                ExportedAnnotation {
                    kind: ExportedAnnotationKind::Line,
                    page: 1,
                    rect: [100.0, 100.0, 200.0, 200.0],
                    name: "pdf-reader-2-1".to_string(),
                    in_reply_to: None,
                    color: Some([255, 0, 0]),
                    interior: None,
                    opacity: None,
                    width: Some(2.5),
                    print: true,
                    hidden: false,
                    icon: None,
                    contents: None,
                    quads: vec![],
                    ink: vec![],
                    line: Some([100.0, 100.0, 200.0, 200.0]),
                    line_endings: Some([ExportedLineEnding::None, ExportedLineEnding::ClosedArrow]),
                    text_style: None,
                },
            ],
            skipped: 1,
            truncated: false,
        };

        let result = file(&annotations, "sample.pdf");
        assert!(result.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n<annots>\n"));
        assert!(result.contains(r##"<highlight page="0" rect="72,700,200,720" flags="print" name="pdf-reader-1-1" color="#FFFF00" opacity="0.8" coords="72,720,200,720,72,700,200,700">"##));
        assert!(result.contains("<contents>Highlight text</contents>"));
        assert!(result.contains(r##"<text page="0" rect="50,650,70,670" flags="print" name="pdf-reader-1-2" color="#0080FF" inreplyto="pdf-reader-1-1" icon="Comment">"##));
        assert!(result.contains("<contents>Reply note</contents>"));
        assert!(result.contains(r##"<line page="1" rect="100,100,200,200" flags="print" name="pdf-reader-2-1" color="#FF0000" width="2.5" start="100,100" end="200,200" head="None" tail="ClosedArrow"/>"##));
        assert!(result.ends_with("</annots>\n<f href=\"sample.pdf\"/>\n</xfdf>\n"));

        // Privacy checks: no author, no dates, no subject, no IDs
        assert!(!result.contains("title="));
        assert!(!result.contains("date="));
        assert!(!result.contains("creationdate="));
        assert!(!result.contains("subject="));
        assert!(!result.contains("<ids"));
    }
}
