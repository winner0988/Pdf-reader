//! A text box through the real worker in the real sandbox (B2-28, ADR 0018): the font arrives as
//! bytes (never a path) to be prepared, the worker reads and cuts it, and the model it gives back
//! is all that is needed to put the words in a box: also in another worker, which never saw the
//! font, as undo and the crash recovery journal need. A font or a text it cannot set is refused
//! with a code of its own. The drawing itself is in `text_box.rs`.
#![cfg(windows)]

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use ipc_contract::types::{
    AnnotationKind, DocumentId, ErrorCode, FontRefusal, Rect, Rgb, TextAlign, TextBoxStyle,
};
use ipc_contract::worker::{
    FontData, PreparedTextBoxModel, TextBoxModel, WorkerEdit, WorkerErrorCode, WorkerRequest,
    WorkerResponse,
};
use worker_host::{HostConfig, HostError, WorkerHost};

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

fn droid() -> FontData {
    FontData {
        bytes: mupdf_fonts_droid::find_by_name("Droid Sans Fallback", false, false)
            .expect("the bundled font")
            .data
            .to_vec(),
        face: 0,
        name: "Droid Sans Fallback".to_owned(),
    }
}

const BOX: Rect = Rect {
    x0: 72.0,
    y0: 72.0,
    x1: 372.0,
    y1: 132.0,
};

const STYLE: TextBoxStyle = TextBoxStyle {
    size: 14.0,
    color: Rgb {
        r: 26,
        g: 51,
        b: 153,
    },
    align: TextAlign::Left,
};

fn opened(host: &mut WorkerHost) -> DocumentId {
    match host.open(&corpus("benign/single-page.pdf")).expect("open") {
        (doc, WorkerResponse::Opened { .. }) => doc,
        other => panic!("{other:?}"),
    }
}

/// The model the worker makes of `text` in `font`, or why it cannot.
fn prepared(host: &mut WorkerHost, text: &str, font: FontData) -> Result<TextBoxModel, HostError> {
    let answer = host.request(|request| WorkerRequest::PrepareTextBox {
        request,
        font,
        text: text.to_owned(),
    })?;
    match answer {
        WorkerResponse::TextBoxPrepared { model, .. } => Ok(model),
        other => panic!("{other:?}"),
    }
}

fn place(
    host: &mut WorkerHost,
    doc: DocumentId,
    model: &TextBoxModel,
) -> Result<WorkerResponse, HostError> {
    host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::PlaceTextBox {
            page: 0,
            rect: BOX,
            model: model.clone(),
            style: STYLE,
        },
    })
}

/// The code the worker refused with.
fn refusal<T: std::fmt::Debug>(answer: Result<T, HostError>) -> WorkerErrorCode {
    match answer {
        Err(HostError::Worker(error)) => error.code,
        other => panic!("{other:?}"),
    }
}

/// The names of the fonts the appearance of the first annotation of the first page uses.
fn appearance_fonts(file: &[u8]) -> Vec<Vec<u8>> {
    let doc = mupdf::pdf::PdfDocument::from_bytes(file).expect("open");
    let page = doc.load_pdf_page(0).expect("page");
    let annotation = page.annotations().next().expect("an annotation");
    let ap = annotation.object().get_dict("AP").unwrap().expect("AP");
    let n = ap.get_dict("N").unwrap().expect("N");
    let fonts = n
        .get_dict("Resources")
        .unwrap()
        .and_then(|resources| resources.get_dict("Font").unwrap())
        .expect("fonts");
    (0..fonts.dict_len().unwrap() as i32)
        .filter_map(|at| fonts.get_dict_val(at).unwrap())
        .filter_map(|font| font.get_dict("BaseFont").unwrap())
        .map(|base| base.as_name().unwrap().to_vec())
        .collect()
}

fn annotations(host: &mut WorkerHost, doc: DocumentId) -> Vec<ipc_contract::types::PageAnnotation> {
    let listed = host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc,
            page_index: 0,
        })
        .expect("annotations");
    let WorkerResponse::PageAnnotations { annotations, .. } = listed else {
        panic!("{listed:?}");
    };
    annotations
}

#[test]
fn words_prepared_from_a_font_become_a_text_box_in_another_worker_that_never_saw_the_font() {
    // The font is up to 64 MiB and is sent once, to prepare the words; what comes back is a few
    // kilobytes, which is what undo and the journal keep.
    let model = prepared(&mut host(), "Hello 你好，世界", droid()).expect("a model");
    assert!(model.subset.len() < 20_000, "{} bytes", model.subset.len());

    let mut other = host();
    let doc = opened(&mut other);
    let made = place(&mut other, doc, &model).expect("the box");
    assert!(matches!(made, WorkerResponse::Edited { .. }), "{made:?}");

    let listed = annotations(&mut other, doc);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind, AnnotationKind::TextBox);
    assert_eq!(listed[0].text.as_deref(), Some("Hello 你好，世界"));

    let path = std::env::temp_dir().join(format!("pdf-worker-{}-text-box.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    other.save(doc, &file).expect("save");
    drop(file);
    let saved = std::fs::read(&path).expect("saved file");
    std::fs::remove_file(&path).ok();
    // A few kilobytes of the font, not its 3.5 MB.
    assert!(saved.len() < 200_000, "{} bytes", saved.len());
    let fonts = appearance_fonts(&saved);
    assert!(!fonts.is_empty());
    assert!(
        fonts.iter().all(|name| name.len() > 7
            && name[6] == b'+'
            && name[..6].iter().all(u8::is_ascii_uppercase)),
        "{fonts:?}"
    );
}

#[test]
fn a_font_or_a_text_the_worker_cannot_set_is_refused_with_a_code_of_its_own() {
    let mut host = host();

    let garbage = FontData {
        bytes: b"not a font at all".to_vec(),
        ..droid()
    };
    let code = refusal(prepared(&mut host, "words", garbage));
    assert_eq!(code, WorkerErrorCode::Font(FontRefusal::NotAFont));
    assert_eq!(ErrorCode::from(code), ErrorCode::FontInvalid);

    let missing = refusal(prepared(&mut host, "words \u{e000}", droid()));
    assert_eq!(ErrorCode::from(missing), ErrorCode::FontMissingGlyphs);

    let arabic = refusal(prepared(&mut host, "\u{628}\u{644}", droid()));
    assert_eq!(arabic, WorkerErrorCode::UnsupportedText);
    assert_eq!(ErrorCode::from(arabic), ErrorCode::TextNotSupported);

    // The worker still answers, and a document of it is as it was.
    let doc = opened(&mut host);
    assert!(annotations(&mut host, doc).is_empty());
}

#[test]
fn a_model_that_was_tampered_with_makes_no_text_box_and_does_not_hurt_the_worker() {
    let mut host = host();
    let model = prepared(&mut host, "Hello", droid()).expect("a model");
    let doc = opened(&mut host);
    let mut garbage = model.clone();
    garbage.subset = [&[0u8, 1, 0, 0][..], &[7u8; 64][..]].concat();
    let mut missing = model.clone();
    missing.glyphs.pop();
    for (what, broken) in [
        ("a subset that is no font", garbage),
        ("a lost glyph", missing),
    ] {
        let code = refusal(place(&mut host, doc, &broken));
        assert!(
            matches!(
                code,
                WorkerErrorCode::InvalidRequest | WorkerErrorCode::Font(FontRefusal::NotAFont)
            ),
            "{what}: {code:?}"
        );
    }
    assert!(annotations(&mut host, doc).is_empty());
    // And it takes the good one.
    place(&mut host, doc, &model).expect("the box");
    assert_eq!(annotations(&mut host, doc).len(), 1);
}

fn info(
    host: &mut WorkerHost,
    doc: DocumentId,
    annotation: ipc_contract::types::AnnotationId,
) -> ipc_contract::types::TextBoxInfo {
    let answer = host
        .request(|request| WorkerRequest::GetTextBox {
            request,
            doc,
            page_index: 0,
            annotation,
        })
        .expect("the text box");
    let WorkerResponse::TextBox { info, .. } = answer else {
        panic!("{answer:?}");
    };
    info
}

fn edit(host: &mut WorkerHost, doc: DocumentId, edit: WorkerEdit) {
    let made = host
        .request(|request| WorkerRequest::Edit { request, doc, edit })
        .expect("the edit");
    assert!(matches!(made, WorkerResponse::Edited { .. }), "{made:?}");
}

#[test]
fn a_text_box_is_read_moved_restyled_and_given_other_words_and_stays_the_same_annotation() {
    let mut host = host();
    let model = prepared(&mut host, "Hello 你好", droid()).expect("a model");
    let doc = opened(&mut host);
    place(&mut host, doc, &model).expect("the box");
    let listed = annotations(&mut host, doc);
    let id = listed[0].id;

    // What the dialog is filled in with.
    let told = info(&mut host, doc, id);
    assert_eq!(told.text, "Hello 你好");
    assert_eq!(told.font, "Droid Sans Fallback");
    assert_eq!(told.style, STYLE);
    assert!(!told.view_only && !told.overflow);

    // Moved: a hundred points down.
    let moved = Rect {
        y0: BOX.y0 + 100.0,
        y1: BOX.y1 + 100.0,
        ..BOX
    };
    edit(
        &mut host,
        doc,
        WorkerEdit::SetAnnotationRect {
            page: 0,
            annotation: id,
            rect: moved,
        },
    );
    let after = annotations(&mut host, doc);
    assert_eq!((after[0].id, after[0].rect.y0), (id, moved.y0));

    // Another size, colour and alignment, with the words and the font it has: no font is sent.
    let red = TextBoxStyle {
        size: 20.0,
        color: Rgb { r: 200, g: 0, b: 0 },
        align: TextAlign::Right,
    };
    edit(
        &mut host,
        doc,
        WorkerEdit::SetTextBox {
            page: 0,
            annotation: id,
            model: None,
            style: red,
        },
    );
    let told = info(&mut host, doc, id);
    assert_eq!((told.text.as_str(), told.style), ("Hello 你好", red));

    // Other words, prepared from the font.
    let other = prepared(&mut host, "Goodbye 再見", droid()).expect("a model");
    edit(
        &mut host,
        doc,
        WorkerEdit::SetTextBox {
            page: 0,
            annotation: id,
            model: Some(other),
            style: red,
        },
    );
    let finally = annotations(&mut host, doc);
    assert_eq!((finally.len(), finally[0].id), (1, id));
    assert_eq!(finally[0].text.as_deref(), Some("Goodbye 再見"));
    assert_eq!(
        finally[0].rect.y0, moved.y0,
        "and it stayed where it was put"
    );
}

#[test]
fn a_font_inspected_through_the_worker_gives_its_faces_and_refuses_bad_fonts() {
    let mut host = host();
    let answer = host
        .inspect_font(&droid().bytes)
        .expect("inspect Droid font");
    let WorkerResponse::FontInspected { faces, .. } = answer else {
        panic!("unexpected response: {answer:?}");
    };
    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0].family, "Droid Sans Fallback");
    assert_eq!(faces[0].name, "Droid Sans Fallback");
    assert_eq!(faces[0].face, 0);

    let bad = host.inspect_font(b"not a font");
    assert_eq!(refusal(bad), WorkerErrorCode::Font(FontRefusal::NotAFont));
}

#[test]
fn prepare_text_boxes_batches_models_and_handles_failures() {
    let mut host = host();
    let texts = vec![
        "Hello".to_owned(),
        "你好，世界".to_owned(),
        "مرحبا".to_owned(),
    ];

    let answer = host
        .request(|request| WorkerRequest::PrepareTextBoxes {
            request,
            font: droid(),
            texts: texts.clone(),
        })
        .expect("batch request");

    let WorkerResponse::TextBoxesPrepared { models, .. } = answer else {
        panic!("unexpected response: {answer:?}");
    };

    assert_eq!(models.len(), 3);
    match &models[0] {
        PreparedTextBoxModel::Prepared(model) => assert_eq!(model.text, "Hello"),
        other => panic!("expected Hello to be Prepared, got {other:?}"),
    }
    match &models[1] {
        PreparedTextBoxModel::Prepared(model) => assert_eq!(model.text, "你好，世界"),
        other => panic!("expected Chinese text in Droid to be Prepared, got {other:?}"),
    }
    assert_eq!(
        models[2],
        PreparedTextBoxModel::Refused,
        "Complex script text should be Refused by Droid"
    );

    // Whitespace text rejected upfront by check_prepare_text_boxes_request with InvalidRequest
    let whitespace_res = host.request(|request| WorkerRequest::PrepareTextBoxes {
        request,
        font: droid(),
        texts: vec!["   ".to_owned()],
    });
    match whitespace_res {
        Err(HostError::Worker(err)) => assert_eq!(err.code, WorkerErrorCode::InvalidRequest),
        other => panic!("expected InvalidRequest, got {other:?}"),
    }

    // Empty texts rejected by worker with InvalidRequest
    let empty_res = host.request(|request| WorkerRequest::PrepareTextBoxes {
        request,
        font: droid(),
        texts: Vec::new(),
    });
    match empty_res {
        Err(HostError::Worker(err)) => assert_eq!(err.code, WorkerErrorCode::InvalidRequest),
        other => panic!("expected InvalidRequest, got {other:?}"),
    }

    // Exceeding MAX_PREPARE_TEXT_BOXES rejected with InvalidRequest
    let too_many = vec!["text".to_owned(); ipc_contract::limits::MAX_PREPARE_TEXT_BOXES + 1];
    let too_many_res = host.request(|request| WorkerRequest::PrepareTextBoxes {
        request,
        font: droid(),
        texts: too_many,
    });
    match too_many_res {
        Err(HostError::Worker(err)) => assert_eq!(err.code, WorkerErrorCode::InvalidRequest),
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
}

#[test]
fn prepare_text_boxes_limits_response_to_max_response_bytes() {
    let mut host = host();
    let sample_text: String = (0x4E00u32..)
        .filter_map(char::from_u32)
        .take(1365)
        .collect();
    assert_eq!(sample_text.len(), 4095);

    let texts = vec![sample_text; 200];
    let answer = host
        .request(|request| WorkerRequest::PrepareTextBoxes {
            request,
            font: droid(),
            texts,
        })
        .expect("batch request should succeed within limits");

    let WorkerResponse::TextBoxesPrepared { models, .. } = answer else {
        panic!("unexpected response: {answer:?}");
    };
    assert_eq!(models.len(), 200);

    let mut total_bytes = 0;
    let mut prepared_count = 0;
    let mut deferred_count = 0;
    let mut saw_deferred = false;

    for m in &models {
        match m {
            PreparedTextBoxModel::Prepared(model) => {
                assert!(!saw_deferred, "Prepared model cannot follow Deferred");
                total_bytes += model.subset.len();
                prepared_count += 1;
            }
            PreparedTextBoxModel::Deferred => {
                saw_deferred = true;
                deferred_count += 1;
            }
            PreparedTextBoxModel::Refused => {
                panic!("Droid should be able to shape all characters in sample_text");
            }
        }
    }

    assert!(
        total_bytes <= ipc_contract::limits::MAX_PREPARE_TEXT_BOXES_RESPONSE_BYTES,
        "Total subset bytes {total_bytes} exceeds limit {}",
        ipc_contract::limits::MAX_PREPARE_TEXT_BOXES_RESPONSE_BYTES
    );
    assert!(prepared_count > 0, "At least one model must be prepared");
    assert!(
        deferred_count > 0,
        "Some models must be deferred when batch exceeds response limit"
    );
    assert_eq!(prepared_count + deferred_count, 200);
}
