//! Edits, saves, and copies of an untrusted PDF (Issues #427, #515, ADR 0013):
//! The first 32 bytes of input select 1..=4 typed edits and parameters (main-process validated),
//! apply them in the worker engine, save the document (full rewrite or incremental if signed),
//! re-open and render page 0 at small scale, then execute one secondary operation:
//! SavePages, Revert, PrepareSource + InsertPages, or one of 7 copies (PrivacyCopy, SanitizedCopy,
//! EncryptedCopy, WatermarkCopy, PageMarksCopy, WriteIdentifier, PrepareSignature).
#![no_main]

use std::io::sink;
use std::sync::OnceLock;

use ipc_contract::types::{
    AnnotationId, BookmarkId, Edit, FieldId, HighlightColor, HighlightMark, InkColor, InkWidth,
    PageCorner, PageMarkPages, PageMarkPosition, PageMarks, PageNumberFormat, PageNumberMark,
    Point, Quad, Rect, Restrictions, Rgb, Rotation, SanitizeOptions, Shape, SourceId, StampImageId,
    StampName, TextAlign, TextBoxModelId, TextBoxStyle, TextMarkKind, Watermark, WatermarkContent,
    WatermarkPages, WatermarkPlace,
};
use ipc_contract::validate::Validate;
use ipc_contract::worker::{NewSignature, SigningTime, VisibleSignature, WorkerEdit};
use libfuzzer_sys::arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::{
    PdfDocument, prepare_signature, sample_stamp_png, sample_text_box_model, write_identifier,
};

/// Fixed header length used to pick actions and parameters.
const HEADER_LEN: usize = 32;

fn cached_stamp_png() -> &'static [u8] {
    static PNG: OnceLock<Vec<u8>> = OnceLock::new();
    PNG.get_or_init(sample_stamp_png)
}

fn cached_text_box_model() -> &'static Option<ipc_contract::worker::TextBoxModel> {
    static MODEL: OnceLock<Option<ipc_contract::worker::TextBoxModel>> = OnceLock::new();
    MODEL.get_or_init(|| sample_text_box_model("Fuzz TextBox"))
}

fn sample_shape_rect(u: &mut Unstructured) -> Rect {
    match u.int_in_range(0..=4).unwrap_or(0) {
        0 => Rect {
            x0: 50.0,
            y0: 50.0,
            x1: 200.0,
            y1: 150.0,
        },
        1 => Rect {
            x0: 60.0,
            y0: 60.0,
            x1: 180.0,
            y1: 180.0,
        },
        2 => Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 100.0,
            y1: 100.0,
        },
        3 => Rect {
            x0: 100.0,
            y0: 100.0,
            x1: 50.0,
            y1: 50.0,
        },
        _ => Rect {
            x0: 100.0,
            y0: 100.0,
            x1: 100.0,
            y1: 100.0,
        },
    }
}

fn sample_line_ends(u: &mut Unstructured) -> (Point, Point) {
    match u.int_in_range(0..=4).unwrap_or(0) {
        0 => (Point { x: 72.0, y: 72.0 }, Point { x: 288.0, y: 144.0 }),
        1 => (Point { x: 100.0, y: 200.0 }, Point { x: 300.0, y: 200.0 }),
        2 => (Point { x: 0.0, y: 0.0 }, Point { x: 150.0, y: 150.0 }),
        3 => (Point { x: 100.0, y: 100.0 }, Point { x: 100.0, y: 100.0 }),
        _ => (Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 0.0 }),
    }
}

fn sample_ink_color(u: &mut Unstructured) -> InkColor {
    match u.int_in_range(0..=3).unwrap_or(0) {
        0 => InkColor::Black,
        1 => InkColor::Red,
        2 => InkColor::Blue,
        _ => InkColor::Green,
    }
}

fn sample_ink_width(u: &mut Unstructured) -> InkWidth {
    match u.int_in_range(0..=2).unwrap_or(0) {
        0 => InkWidth::Thin,
        1 => InkWidth::Medium,
        _ => InkWidth::Thick,
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < HEADER_LEN {
        return;
    }
    let (header, pdf_bytes) = data.split_at(HEADER_LEN);
    if pdf_bytes.is_empty() || pdf_bytes.len() > 1_048_576 {
        return;
    }

    let Ok(mut document) = PdfDocument::open(pdf_bytes, None) else {
        return;
    };
    let Ok(page_count) = document.page_count() else {
        return;
    };
    if page_count == 0 || page_count > 100 {
        return;
    }

    let mut u = Unstructured::new(header);
    let secondary = u.int_in_range(0..=9).unwrap_or(0);
    let num_edits = u.int_in_range(1..=4).unwrap_or(1);

    let stamp_png = cached_stamp_png();
    let text_box_model = cached_text_box_model();

    let mut applied_edits = Vec::new();

    for _ in 0..num_edits {
        let kind = u.int_in_range(0..=28).unwrap_or(0);
        let p = (u.int_in_range(0..=255).unwrap_or(0) as u32) % page_count;

        let edit = match kind {
            0 => {
                let by = match u.int_in_range(0..=2).unwrap_or(0) {
                    0 => Rotation::Cw90,
                    1 => Rotation::Cw180,
                    _ => Rotation::Cw270,
                };
                Edit::RotatePages { pages: vec![p], by }
            }
            1 => {
                if page_count > 1 {
                    Edit::DeletePages { pages: vec![p] }
                } else {
                    Edit::RotatePages {
                        pages: vec![0],
                        by: Rotation::Cw90,
                    }
                }
            }
            2 => {
                let before = (u.int_in_range(0..=255).unwrap_or(0) as u32) % (page_count + 1);
                Edit::MovePages {
                    pages: vec![p],
                    before,
                }
            }
            3 => {
                let at = (u.int_in_range(0..=255).unwrap_or(0) as u32) % (page_count + 1);
                Edit::InsertBlankPage { at, like: p }
            }
            4 => {
                let at = (u.int_in_range(0..=255).unwrap_or(0) as u32) % (page_count + 1);
                Edit::InsertPages {
                    at,
                    source: SourceId(1),
                }
            }
            5 => {
                let mark_kind = match u.int_in_range(0..=3).unwrap_or(0) {
                    0 => TextMarkKind::Highlight,
                    1 => TextMarkKind::Underline,
                    2 => TextMarkKind::StrikeOut,
                    _ => TextMarkKind::Squiggly,
                };
                let color = match u.int_in_range(0..=3).unwrap_or(0) {
                    0 => HighlightColor::Yellow,
                    1 => HighlightColor::Green,
                    2 => HighlightColor::Blue,
                    _ => HighlightColor::Pink,
                };
                let mark = HighlightMark {
                    page: p,
                    quads: vec![Quad {
                        ul: Point { x: 72.0, y: 72.0 },
                        ur: Point { x: 200.0, y: 72.0 },
                        ll: Point { x: 72.0, y: 84.0 },
                        lr: Point { x: 200.0, y: 84.0 },
                    }],
                };
                Edit::AddHighlight {
                    mark_kind,
                    marks: vec![mark],
                    color,
                }
            }
            6 => Edit::AddNote {
                page: p,
                at: Point { x: 72.0, y: 72.0 },
                text: "Fuzz note".to_string(),
            },
            7 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                Edit::DeleteAnnotation {
                    page: p,
                    annotation: AnnotationId(id),
                }
            }
            8 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                let color = match u.int_in_range(0..=3).unwrap_or(0) {
                    0 => HighlightColor::Yellow,
                    1 => HighlightColor::Green,
                    2 => HighlightColor::Blue,
                    _ => HighlightColor::Pink,
                };
                Edit::SetHighlightColor {
                    page: p,
                    annotation: AnnotationId(id),
                    color,
                }
            }
            9 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                Edit::SetNoteText {
                    page: p,
                    annotation: AnnotationId(id),
                    text: "Edited note".to_string(),
                }
            }
            10 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                Edit::SetFieldValue {
                    page: p,
                    field: FieldId(id),
                    value: "Value".to_string(),
                }
            }
            11 => Edit::FlattenForm,
            12 => Edit::AddRedaction {
                page: p,
                rect: Rect {
                    x0: 50.0,
                    y0: 50.0,
                    x1: 150.0,
                    y1: 100.0,
                },
            },
            13 => Edit::AddRedactionGroup {
                page: p,
                rects: vec![Rect {
                    x0: 50.0,
                    y0: 50.0,
                    x1: 150.0,
                    y1: 100.0,
                }],
            },
            14 => Edit::ApplyRedactions,
            15 => {
                let color = match u.int_in_range(0..=3).unwrap_or(0) {
                    0 => InkColor::Black,
                    1 => InkColor::Red,
                    2 => InkColor::Blue,
                    _ => InkColor::Green,
                };
                let width = match u.int_in_range(0..=2).unwrap_or(0) {
                    0 => InkWidth::Thin,
                    1 => InkWidth::Medium,
                    _ => InkWidth::Thick,
                };
                Edit::AddInk {
                    page: p,
                    strokes: vec![vec![
                        Point { x: 100.0, y: 100.0 },
                        Point { x: 150.0, y: 150.0 },
                    ]],
                    color,
                    width,
                }
            }
            16 => {
                let stamp = match u.int_in_range(0..=7).unwrap_or(0) {
                    0 => StampName::Approved,
                    1 => StampName::NotApproved,
                    2 => StampName::Draft,
                    3 => StampName::Final,
                    4 => StampName::Confidential,
                    5 => StampName::ForComment,
                    6 => StampName::AsIs,
                    _ => StampName::TopSecret,
                };
                Edit::AddStamp {
                    page: p,
                    rect: Rect {
                        x0: 100.0,
                        y0: 100.0,
                        x1: 200.0,
                        y1: 150.0,
                    },
                    stamp,
                }
            }
            17 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                Edit::SetAnnotationRect {
                    page: p,
                    annotation: AnnotationId(id),
                    rect: Rect {
                        x0: 50.0,
                        y0: 50.0,
                        x1: 150.0,
                        y1: 100.0,
                    },
                }
            }
            18 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                let deg = [0.0, 90.0, 180.0, 270.0][(u.int_in_range(0..=3).unwrap_or(0)) as usize];
                Edit::SetPictureAngle {
                    page: p,
                    annotation: AnnotationId(id),
                    degrees: deg,
                }
            }
            19 => Edit::AddImageStamp {
                page: p,
                rect: Rect {
                    x0: 100.0,
                    y0: 100.0,
                    x1: 200.0,
                    y1: 150.0,
                },
                image: StampImageId(1),
            },
            20 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                let style = TextBoxStyle {
                    size: 14.0,
                    color: Rgb { r: 0, g: 0, b: 0 },
                    align: TextAlign::Left,
                };
                Edit::SetTextBox {
                    page: p,
                    annotation: AnnotationId(id),
                    model: Some(TextBoxModelId(1)),
                    style,
                }
            }
            21 => {
                let style = TextBoxStyle {
                    size: 14.0,
                    color: Rgb { r: 0, g: 0, b: 0 },
                    align: TextAlign::Left,
                };
                Edit::AddTextBox {
                    page: p,
                    rect: Rect {
                        x0: 100.0,
                        y0: 100.0,
                        x1: 300.0,
                        y1: 200.0,
                    },
                    model: TextBoxModelId(1),
                    style,
                }
            }
            22 => Edit::AddBookmark {
                parent: None,
                index: 0,
                title: "Bookmark".to_string(),
                page: p,
            },
            23 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                Edit::RenameBookmark {
                    id: BookmarkId(id),
                    title: "Renamed".to_string(),
                }
            }
            24 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                Edit::DeleteBookmark { id: BookmarkId(id) }
            }
            25 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                Edit::MoveBookmark {
                    id: BookmarkId(id),
                    parent: None,
                    index: 0,
                }
            }
            26 => {
                let color = sample_ink_color(&mut u);
                let width = sample_ink_width(&mut u);
                let shape = match u.int_in_range(0..=3).unwrap_or(0) {
                    0 => Shape::Rectangle {
                        rect: sample_shape_rect(&mut u),
                    },
                    1 => Shape::Ellipse {
                        rect: sample_shape_rect(&mut u),
                    },
                    2 => {
                        let (from, to) = sample_line_ends(&mut u);
                        Shape::Line {
                            from,
                            to,
                            arrow: false,
                        }
                    }
                    _ => {
                        let (from, to) = sample_line_ends(&mut u);
                        Shape::Line {
                            from,
                            to,
                            arrow: true,
                        }
                    }
                };
                Edit::AddShape {
                    page: p,
                    shape,
                    color,
                    width,
                }
            }
            27 => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                let (from, to) = sample_line_ends(&mut u);
                Edit::SetLineEnds {
                    page: p,
                    annotation: AnnotationId(id),
                    from,
                    to,
                }
            }
            _ => {
                let id = u.int_in_range(0..=10).unwrap_or(0);
                let color = sample_ink_color(&mut u);
                let width = sample_ink_width(&mut u);
                Edit::SetShapeStyle {
                    page: p,
                    annotation: AnnotationId(id),
                    color,
                    width,
                }
            }
        };

        if edit.validate().is_err() {
            continue;
        }

        let w_edit = match WorkerEdit::of(
            &edit,
            |_source_id| Some((pdf_bytes.to_vec(), vec![])),
            |_image_id| Some(stamp_png.to_vec()),
            |_model_id| text_box_model.clone(),
        ) {
            Ok(w) => w,
            Err(_) => continue,
        };

        let _ = pdf_worker::serve::apply(&mut document, &w_edit);
        applied_edits.push(w_edit);
    }

    // Save and re-open to render
    let mut saved = Vec::new();
    if document.save_checked(&mut saved).is_ok() {
        if let Ok(again) = PdfDocument::from_bytes(&saved) {
            let _ = again.render(0, 0.1, 0);
        }
    }

    // Execute selected secondary action
    match secondary {
        0 => {
            let pages_to_save: Vec<u32> = (0..page_count.min(2)).collect();
            let _ = document.pages_copy(&pages_to_save, &mut sink());
        }
        1 => {
            if let Ok(mut reverted) = PdfDocument::open(pdf_bytes, None) {
                for edit in &applied_edits {
                    let _ = pdf_worker::serve::apply(&mut reverted, edit);
                }
                let _ = reverted.save_checked(&mut sink());
            }
        }
        2 => {
            let id = [0x42u8; 16];
            let _ = document.privacy_copy(&id, &mut sink());
        }
        3 => {
            let id = [0x43u8; 16];
            let _ = document.sanitized_copy(&id, SanitizeOptions::default(), &mut sink());
        }
        4 => {
            let restrictions = Restrictions::default();
            let _ = document.encrypted_copy(Some("user"), "owner", restrictions, &mut sink());
        }
        5 => {
            let watermark = Watermark {
                content: WatermarkContent::Text {
                    text: "CONFIDENTIAL".to_string(),
                    size: 36.0,
                    color: Rgb {
                        r: 128,
                        g: 128,
                        b: 128,
                    },
                },
                opacity: 50,
                diagonal: true,
                place: WatermarkPlace::Center,
                pages: WatermarkPages::All,
            };
            let _ = document.watermark_copy(&watermark, None, &mut sink());
        }
        6 => {
            let marks = PageMarks {
                page_number: Some(PageNumberMark {
                    format: PageNumberFormat::PageOfTotal,
                    start: 1,
                    position: PageMarkPosition::BottomCenter,
                }),
                bates: None,
                header_footer: None,
                font_size: 10.0,
                color: Rgb { r: 0, g: 0, b: 0 },
                margin_pt: 36.0,
                pages: PageMarkPages::All,
            };
            let _ = document.page_marks_copy(&marks, &mut sink());
        }
        7 => {
            let id = [0x44u8; 16];
            let _ = write_identifier(pdf_bytes, None, Some(&id), &mut sink());
        }
        8 => {
            let signature = NewSignature {
                signer: "Signer".to_string(),
                reason: Some("Fuzz".to_string()),
                time: SigningTime {
                    year: 2026,
                    month: 11,
                    day: 5,
                    hour: 12,
                    minute: 0,
                    second: 0,
                    utc_offset_minutes: 0,
                },
                visible: Some(VisibleSignature {
                    page_index: 0,
                    corner: PageCorner::BottomRight,
                    lines: vec!["Digitally signed".to_string()],
                }),
            };
            let _ = prepare_signature(pdf_bytes, &signature, &mut sink());
        }
        _ => {
            if let Ok(source) = PdfDocument::prepare_source(pdf_bytes, None) {
                let _ = document.insert_pages(0, &source.bytes, &[]);
                let _ = document.save_checked(&mut sink());
            }
        }
    }
});
