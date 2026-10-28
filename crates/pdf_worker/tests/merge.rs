//! The pages of another file going into a document, through the real worker in the real sandbox
//! (B2-06): the worker reads the file through a read-only handle (never a path), gives back a
//! plain copy of it, and later takes the pages from that copy.
#![cfg(windows)]

use std::fs::File;
use std::path::{Path, PathBuf};

use ipc_contract::limits::{MAX_INK_STROKES, MAX_SOURCE_BYTES, MAX_SOURCE_TEXT_BOXES};
use ipc_contract::types::{
    AnnotationKind, DocumentId, ErrorCode, HighlightColor, HighlightMark, InkColor, InkWidth,
    PageSize, Password, Point, Quad, Rect, Rgb, StampName, TextAlign, TextBoxStyle,
};
use ipc_contract::worker::{
    FontData, InsertedTextBox, WorkerEdit, WorkerErrorCode, WorkerRequest, WorkerResponse,
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

fn opened(host: &mut WorkerHost, path: &Path) -> (DocumentId, Vec<PageSize>) {
    match host.open(path).expect("open") {
        (doc, WorkerResponse::Opened { document, .. }) => (doc, document.pages),
        other => panic!("{other:?}"),
    }
}

/// What the worker makes of the corpus file `name`: its plain copy, its page count and what it
/// has that is active.
fn source(
    host: &mut WorkerHost,
    name: &str,
    password: Option<&str>,
) -> Result<(Vec<u8>, u32, usize), HostError> {
    let file = File::open(corpus(name)).expect("the sample");
    match host.prepare_source(&file, password.map(|text| Password::new(text.to_owned())))? {
        WorkerResponse::Source {
            bytes,
            pages,
            security,
            ..
        } => Ok((bytes, pages, security.findings.len())),
        other => panic!("{other:?}"),
    }
}

fn code(error: &HostError) -> ErrorCode {
    error.code()
}

#[test]
fn the_pages_of_an_encrypted_file_go_into_a_document_by_a_plain_copy() {
    let mut host = host();
    // The file needs its password, and says so; a wrong one is no better.
    assert_eq!(
        code(&source(&mut host, "benign/encrypted-aes256.pdf", None).unwrap_err()),
        ErrorCode::Encrypted
    );
    assert_eq!(
        code(&source(&mut host, "benign/encrypted-aes256.pdf", Some("wrong")).unwrap_err()),
        ErrorCode::Encrypted
    );
    let (bytes, pages, findings) =
        source(&mut host, "benign/encrypted-aes256.pdf", Some("user")).expect("source");
    assert_eq!((pages, findings), (1, 0));
    // Plain: nothing in it says it is encrypted, so it needs no password later.
    assert!(!bytes.windows(8).any(|window| window == b"/Encrypt"));

    let (doc, before) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    assert_eq!(before.len(), 10);
    let edited = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::InsertPages {
                at: 2,
                source: bytes.clone(),
                text_boxes: Vec::new(),
            },
        })
        .expect("insert");
    let WorkerResponse::Edited { pages, .. } = edited else {
        panic!("{edited:?}");
    };
    assert_eq!(pages.len(), 11);
    // The same edit, made again by undo (Revert opens the document again from its bytes), and
    // the same source: the pages are there again.
    let reverted = host
        .request(|request| WorkerRequest::Revert {
            request,
            doc,
            edits: vec![WorkerEdit::InsertPages {
                at: 2,
                source: bytes,
                text_boxes: Vec::new(),
            }],
            password: None,
        })
        .expect("revert");
    assert!(matches!(
        reverted,
        WorkerResponse::Edited { pages, .. } if pages.len() == 11
    ));
}

#[test]
fn a_file_that_may_not_be_taken_from_a_file_that_is_no_pdf_and_one_too_large_are_refused() {
    let mut host = host();
    // The author forbids copying: the pages cannot be taken out.
    let refused = source(&mut host, "benign/restricted-no-copy-no-print.pdf", None).unwrap_err();
    assert_eq!(code(&refused), ErrorCode::NotAllowed);
    assert!(matches!(
        refused,
        HostError::Worker(error) if error.code == WorkerErrorCode::NotAllowed
    ));
    // The owner password lifts that.
    assert!(
        source(
            &mut host,
            "benign/restricted-open-password.pdf",
            Some("owner")
        )
        .is_ok()
    );

    // Not a PDF.
    let error = source(&mut host, "manifest.json", None);
    assert!(error.is_err());

    // Larger than a source may be: refused before the worker reads a byte of it.
    let path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-large-source.pdf",
        std::process::id()
    ));
    let large = File::create(&path).expect("create");
    large.set_len(MAX_SOURCE_BYTES as u64 + 1).expect("sparse");
    drop(large);
    let file = File::open(&path).expect("open");
    let error = host.prepare_source(&file, None).unwrap_err();
    std::fs::remove_file(&path).ok();
    assert!(matches!(error, HostError::TooLarge));

    // The worker goes on after any of it.
    let (_, pages, _) = source(&mut host, "benign/mixed-page-sizes.pdf", None).expect("source");
    assert_eq!(pages, 4);
}

#[test]
fn what_is_active_in_a_file_is_reported_and_stays_out_of_the_document() {
    let mut host = host();
    let (doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    let (bytes, pages, findings) =
        source(&mut host, "malicious/openaction-js.pdf", None).expect("source");
    // The scan sees it (so that the banner can say so) ...
    assert!(findings > 0);
    // ... and the pages come without it.
    let edited = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::InsertPages {
                at: 10,
                source: bytes,
                text_boxes: Vec::new(),
            },
        })
        .expect("insert");
    assert!(matches!(
        edited,
        WorkerResponse::Edited { pages: ref all, .. } if all.len() as u32 == 10 + pages
    ));
    // Saved, and opened again: what it says of active content is nothing.
    let path = std::env::temp_dir().join(format!("pdf-worker-{}-merged.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    host.save(doc, &file).expect("save");
    drop(file);
    let (_, resulting) = match host.open(&path) {
        Ok((doc, WorkerResponse::Opened { document, .. })) => (doc, document),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&path).ok();
    assert_eq!(resulting.pages.len() as u32, 10 + pages);
    assert!(resulting.security.findings.is_empty());
}

#[test]
fn app_annotations_are_recreated_on_inserted_pages() {
    let mut host = host();
    // 1. Prepare a source document with the 5 supported annotation kinds.
    let (source_doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));

    // Highlight
    let quad = Quad {
        ul: Point { x: 72.0, y: 72.0 },
        ur: Point { x: 144.0, y: 72.0 },
        ll: Point { x: 72.0, y: 84.0 },
        lr: Point { x: 144.0, y: 84.0 },
    };
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddHighlight {
            marks: vec![HighlightMark {
                page: 0,
                quads: vec![quad],
            }],
            color: HighlightColor::Yellow,
        },
    })
    .expect("highlight");

    // Note
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddNote {
            page: 0,
            at: Point { x: 100.0, y: 200.0 },
            text: "Source Note".to_owned(),
        },
    })
    .expect("note");

    // Ink
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddInk {
            page: 0,
            strokes: vec![vec![Point { x: 50.0, y: 50.0 }, Point { x: 60.0, y: 60.0 }]],
            color: InkColor::Red,
            width: InkWidth::Medium,
        },
    })
    .expect("ink");

    // Standard Stamp
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddStamp {
            page: 0,
            rect: Rect {
                x0: 200.0,
                y0: 200.0,
                x1: 300.0,
                y1: 250.0,
            },
            stamp: StampName::Approved,
        },
    })
    .expect("standard stamp");

    // Picture Stamp
    let pic_file = File::open(corpus("images/stamp-exif.jpg")).expect("pic file");
    let png = match host.prepare_stamp_image(&pic_file).expect("prepare stamp") {
        WorkerResponse::StampImage { png, .. } => png,
        other => panic!("{other:?}"),
    };
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddImageStamp {
            page: 0,
            rect: Rect {
                x0: 300.0,
                y0: 300.0,
                x1: 400.0,
                y1: 350.0,
            },
            png,
        },
    })
    .expect("picture stamp");

    // Text Box
    let model = match host
        .request(|request| WorkerRequest::PrepareTextBox {
            request,
            font: droid(),
            text: "Source Text Box".to_owned(),
        })
        .expect("prepare text box")
    {
        WorkerResponse::TextBoxPrepared { model, .. } => model,
        other => panic!("{other:?}"),
    };
    let tb_style = TextBoxStyle {
        size: 14.0,
        color: Rgb {
            r: 26,
            g: 51,
            b: 153,
        },
        align: TextAlign::Left,
    };
    let tb_rect = Rect {
        x0: 100.0,
        y0: 400.0,
        x1: 300.0,
        y1: 460.0,
    };
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::PlaceTextBox {
            page: 0,
            rect: tb_rect,
            model: model.clone(),
            style: tb_style,
        },
    })
    .expect("text box");

    // Save source document to a temp file
    let source_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-annot-source.pdf",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&source_path);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&source_path)
        .expect("create source file");
    host.save(source_doc, &file).expect("save source doc");
    drop(file);

    // 2. Prepare source from the saved file
    let file = File::open(&source_path).expect("open source file");
    let (source_bytes, source_pages, source_tb) =
        match host.prepare_source(&file, None).expect("prepare source") {
            WorkerResponse::Source {
                bytes,
                pages,
                text_boxes,
                ..
            } => (bytes, pages, text_boxes),
            other => panic!("{other:?}"),
        };
    std::fs::remove_file(&source_path).ok();
    assert_eq!(source_pages, 1);
    assert_eq!(source_tb.len(), 1);
    assert_eq!(source_tb[0].text, "Source Text Box");

    // 3. Open target doc (multi-page-10.pdf) and insert source at page 2
    let (target_doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    let inserted_tb = InsertedTextBox {
        page: 0,
        rect: tb_rect,
        model,
        style: tb_style,
    };
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 2,
            source: source_bytes.clone(),
            text_boxes: vec![inserted_tb.clone()],
        },
    })
    .expect("insert pages");

    // 4. Check annotations on the newly inserted page (page 2)
    let annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };

    // Expect 6 annotations: Highlight, Note, Ink, Standard Stamp, Picture Stamp, Text Box!
    assert_eq!(
        annots.len(),
        6,
        "Expected 6 annotations on inserted page, found: {annots:?}"
    );
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::Highlight));
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::Note));
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::Ink));
    assert!(
        annots
            .iter()
            .any(|a| a.kind == AnnotationKind::Stamp && a.picture.is_none())
    );
    assert!(
        annots
            .iter()
            .any(|a| a.kind == AnnotationKind::Stamp && a.picture.is_some())
    );
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::TextBox));

    // 5. Revert (undo/redo & crash recovery replay) with the same edit
    let reverted = host
        .request(|request| WorkerRequest::Revert {
            request,
            doc: target_doc,
            edits: vec![WorkerEdit::InsertPages {
                at: 2,
                source: source_bytes.clone(),
                text_boxes: vec![inserted_tb.clone()],
            }],
            password: None,
        })
        .expect("revert");
    assert!(matches!(reverted, WorkerResponse::Edited { .. }));

    let reverted_annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots after revert")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };
    assert_eq!(reverted_annots.len(), 6);
}

#[test]
fn malicious_actions_on_source_annotations_are_stripped() {
    use mupdf::pdf::PdfObject;

    let mut host = host();
    // 1. Create a source file with an annotation
    let (source_doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));
    let quad = Quad {
        ul: Point { x: 72.0, y: 72.0 },
        ur: Point { x: 144.0, y: 72.0 },
        ll: Point { x: 72.0, y: 84.0 },
        lr: Point { x: 144.0, y: 84.0 },
    };
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddHighlight {
            marks: vec![HighlightMark {
                page: 0,
                quads: vec![quad],
            }],
            color: HighlightColor::Yellow,
        },
    })
    .expect("add highlight");

    let source_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-malicious-annot.pdf",
        std::process::id()
    ));
    let file = File::create(&source_path).expect("create file");
    host.save(source_doc, &file).expect("save file");
    drop(file);

    // 2. Inject /A JavaScript action directly into the saved PDF's annotation
    let modified_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-malicious-annot-mod.pdf",
        std::process::id()
    ));
    {
        let doc = mupdf::pdf::PdfDocument::open(source_path.to_str().unwrap()).expect("open mupdf");
        let page = doc.load_pdf_page(0).expect("load page");
        let annot = page.annotations().next().expect("annot");
        let mut annot_obj = annot.object();
        let mut action = doc.new_dict().expect("dict");
        action
            .dict_put("S", PdfObject::new_name("JavaScript").expect("name"))
            .expect("put");
        action
            .dict_put(
                "JS",
                PdfObject::new_string("app.alert('evil_pwn')").expect("string"),
            )
            .expect("put");
        annot_obj.dict_put("A", action).expect("put A");

        let mut aa_dict = doc.new_dict().expect("dict");
        let mut aa_action = doc.new_dict().expect("dict");
        aa_action
            .dict_put("S", PdfObject::new_name("JavaScript").expect("name"))
            .expect("put");
        aa_action
            .dict_put(
                "JS",
                PdfObject::new_string("app.alert('evil_aa')").expect("string"),
            )
            .expect("put");
        aa_dict.dict_put("PO", aa_action).expect("put PO");
        annot_obj.dict_put("AA", aa_dict).expect("put AA");

        let mut launch_action = doc.new_dict().expect("dict");
        launch_action
            .dict_put("S", PdfObject::new_name("Launch").expect("name"))
            .expect("put");
        launch_action
            .dict_put("F", PdfObject::new_string("evil_cmd.exe").expect("string"))
            .expect("put");
        annot_obj
            .dict_put("Launch", launch_action)
            .expect("put Launch");

        let mut popup_dict = doc.new_dict().expect("dict");
        popup_dict
            .dict_put("Type", PdfObject::new_name("Annot").expect("name"))
            .expect("put");
        popup_dict
            .dict_put("Subtype", PdfObject::new_name("Popup").expect("name"))
            .expect("put");
        let mut popup_act = doc.new_dict().expect("dict");
        popup_act
            .dict_put("S", PdfObject::new_name("JavaScript").expect("name"))
            .expect("put");
        popup_act
            .dict_put(
                "JS",
                PdfObject::new_string("app.alert('evil_popup')").expect("string"),
            )
            .expect("put");
        popup_dict.dict_put("A", popup_act).expect("put popup A");
        annot_obj.dict_put("Popup", popup_dict).expect("put Popup");

        doc.save_with_options(
            modified_path.to_str().unwrap(),
            mupdf::pdf::PdfWriteOptions::default(),
        )
        .expect("save");
    }
    std::fs::remove_file(&source_path).ok();

    // 3. Prepare source from this modified file
    let file = File::open(&modified_path).expect("open file");
    let (source_bytes, _, _) = match host.prepare_source(&file, None).expect("prepare source") {
        WorkerResponse::Source {
            bytes,
            pages,
            text_boxes,
            ..
        } => (bytes, pages, text_boxes),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&modified_path).ok();

    // 4. Insert into target document
    let (target_doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 2,
            source: source_bytes,
            text_boxes: Vec::new(),
        },
    })
    .expect("insert pages");

    // 5. Verify highlight was recreated on page 2
    let annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };

    assert_eq!(annots.len(), 1);
    assert_eq!(annots[0].kind, AnnotationKind::Highlight);

    // 6. Save target doc and verify absence of evil strings
    let merged_path =
        std::env::temp_dir().join(format!("pdf-worker-{}-merged-evil.pdf", std::process::id()));
    let file = File::create(&merged_path).expect("create");
    host.save(target_doc, &file).expect("save");
    drop(file);

    let saved_bytes = std::fs::read(&merged_path).expect("read");
    std::fs::remove_file(&merged_path).ok();

    assert!(!saved_bytes.windows(8).any(|w| w == b"evil_pwn"));
    assert!(!saved_bytes.windows(7).any(|w| w == b"evil_aa"));
    assert!(!saved_bytes.windows(12).any(|w| w == b"evil_cmd.exe"));
    assert!(!saved_bytes.windows(10).any(|w| w == b"evil_popup"));
    assert!(!saved_bytes.windows(11).any(|w| w == b"/JavaScript"));
    assert!(!saved_bytes.windows(7).any(|w| w == b"/Launch"));
    assert!(!saved_bytes.windows(3).any(|w| w == b"/AA"));
    assert!(!saved_bytes.windows(6).any(|w| w == b"/Popup"));
}

#[test]
fn invalid_annotations_are_skipped_while_valid_are_kept() {
    use mupdf::pdf::PdfObject;

    let mut host = host();
    // 1. Create source file with valid highlight, note, stamp
    let (source_doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));
    let quad = Quad {
        ul: Point { x: 72.0, y: 72.0 },
        ur: Point { x: 144.0, y: 72.0 },
        ll: Point { x: 72.0, y: 84.0 },
        lr: Point { x: 144.0, y: 84.0 },
    };
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddHighlight {
            marks: vec![HighlightMark {
                page: 0,
                quads: vec![quad],
            }],
            color: HighlightColor::Yellow,
        },
    })
    .expect("add valid highlight");

    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddNote {
            page: 0,
            at: Point { x: 100.0, y: 200.0 },
            text: "Valid Note".to_owned(),
        },
    })
    .expect("add valid note");

    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddStamp {
            page: 0,
            rect: Rect {
                x0: 200.0,
                y0: 200.0,
                x1: 300.0,
                y1: 250.0,
            },
            stamp: StampName::Approved,
        },
    })
    .expect("add valid stamp");

    let source_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-invalid-annots.pdf",
        std::process::id()
    ));
    let file = File::create(&source_path).expect("create file");
    host.save(source_doc, &file).expect("save file");
    drop(file);

    // 2. Inject invalid annotations:
    //    - invalid stamp with unrecognized Name
    //    - invalid note with text exceeding MAX_NOTE_TEXT_BYTES
    //    - invalid highlight with unrecognized color
    let modified_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-invalid-annots-mod.pdf",
        std::process::id()
    ));
    {
        let doc = mupdf::pdf::PdfDocument::open(source_path.to_str().unwrap()).expect("open mupdf");
        let mut page = doc.load_pdf_page(0).expect("load page");

        // Invalid stamp
        let invalid_stamp = page
            .create_annotation(mupdf::pdf::PdfAnnotationType::Stamp)
            .expect("create stamp");
        let mut stamp_obj = invalid_stamp.object();
        stamp_obj
            .dict_put(
                "Name",
                PdfObject::new_name("NotAnAppStampName").expect("name"),
            )
            .expect("put name");
        let mut rect = doc.new_array().expect("array");
        for v in [50.0, 50.0, 100.0, 100.0] {
            rect.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        stamp_obj.dict_put("Rect", rect).expect("put rect");

        // Invalid note (text exceeds MAX_NOTE_TEXT_BYTES)
        let invalid_note = page
            .create_annotation(mupdf::pdf::PdfAnnotationType::Text)
            .expect("create note");
        let mut note_obj = invalid_note.object();
        let long_text = "A".repeat(100_000);
        note_obj
            .dict_put(
                "Contents",
                PdfObject::new_string(&long_text).expect("string"),
            )
            .expect("put contents");
        let mut rect = doc.new_array().expect("array");
        for v in [60.0, 60.0, 80.0, 80.0] {
            rect.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        note_obj.dict_put("Rect", rect).expect("put rect");

        // Invalid highlight (unrecognized color [0.123, 0.456, 0.789])
        let invalid_hl = page
            .create_annotation(mupdf::pdf::PdfAnnotationType::Highlight)
            .expect("create hl");
        let mut hl_obj = invalid_hl.object();
        let mut color = doc.new_array().expect("array");
        for v in [0.123, 0.456, 0.789] {
            color
                .array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        hl_obj.dict_put("C", color).expect("put color");
        let mut qp = doc.new_array().expect("array");
        for v in [10.0, 10.0, 50.0, 10.0, 10.0, 20.0, 50.0, 20.0] {
            qp.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        hl_obj.dict_put("QuadPoints", qp).expect("put quad");
        let mut rect = doc.new_array().expect("array");
        for v in [10.0, 10.0, 50.0, 20.0] {
            rect.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        hl_obj.dict_put("Rect", rect).expect("put rect");

        doc.save_with_options(
            modified_path.to_str().unwrap(),
            mupdf::pdf::PdfWriteOptions::default(),
        )
        .expect("save");
    }
    std::fs::remove_file(&source_path).ok();

    // 3. Prepare source from this modified file
    let file = File::open(&modified_path).expect("open file");
    let (source_bytes, _, _) = match host.prepare_source(&file, None).expect("prepare source") {
        WorkerResponse::Source {
            bytes,
            pages,
            text_boxes,
            ..
        } => (bytes, pages, text_boxes),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&modified_path).ok();

    // 4. Insert into target document
    let (target_doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 2,
            source: source_bytes,
            text_boxes: Vec::new(),
        },
    })
    .expect("insert pages");

    // 5. Verify ONLY the 3 valid annotations are recreated on page 2!
    let annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };

    assert_eq!(
        annots.len(),
        3,
        "Expected only 3 valid annotations, got: {annots:?}"
    );
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::Highlight));
    assert!(
        annots
            .iter()
            .any(|a| a.kind == AnnotationKind::Note && a.text.as_deref() == Some("Valid Note"))
    );
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::Stamp));
}

#[test]
fn rotated_source_page_preserves_annotation_geometry() {
    let mut host = host();
    // 1. Create a source file, rotate page by 90 degrees, and add a note & highlight
    let (source_doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::RotatePages {
            pages: vec![0],
            degrees: 90,
        },
    })
    .expect("rotate source page");

    let quad = Quad {
        ul: Point { x: 100.0, y: 150.0 },
        ur: Point { x: 200.0, y: 150.0 },
        ll: Point { x: 100.0, y: 170.0 },
        lr: Point { x: 200.0, y: 170.0 },
    };
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddHighlight {
            marks: vec![HighlightMark {
                page: 0,
                quads: vec![quad],
            }],
            color: HighlightColor::Yellow,
        },
    })
    .expect("add highlight");

    let note_at = Point { x: 120.0, y: 250.0 };
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: source_doc,
        edit: WorkerEdit::AddNote {
            page: 0,
            at: note_at,
            text: "Rotated Page Note".to_owned(),
        },
    })
    .expect("add note");

    let source_annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: source_doc,
            page_index: 0,
        })
        .expect("source page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };
    let source_hl = source_annots
        .iter()
        .find(|a| a.kind == AnnotationKind::Highlight)
        .expect("source hl")
        .clone();
    let source_note = source_annots
        .iter()
        .find(|a| a.kind == AnnotationKind::Note)
        .expect("source note")
        .clone();

    let source_path =
        std::env::temp_dir().join(format!("pdf-worker-{}-rotated.pdf", std::process::id()));
    let file = File::create(&source_path).expect("create file");
    host.save(source_doc, &file).expect("save file");
    drop(file);

    // 2. Prepare source from this file
    let file = File::open(&source_path).expect("open file");
    let (source_bytes, pages, _) = match host.prepare_source(&file, None).expect("prepare source") {
        WorkerResponse::Source {
            bytes,
            pages,
            text_boxes,
            ..
        } => (bytes, pages, text_boxes),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&source_path).ok();
    assert_eq!(pages, 1);

    // 3. Insert into target document
    let (target_doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    let edited = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc: target_doc,
            edit: WorkerEdit::InsertPages {
                at: 2,
                source: source_bytes,
                text_boxes: Vec::new(),
            },
        })
        .expect("insert pages");

    let WorkerResponse::Edited {
        pages: all_pages, ..
    } = edited
    else {
        panic!("{edited:?}");
    };
    // Check that inserted page retained 90 degree orientation (dimensions swapped)
    assert_eq!(all_pages[2].width_pt, 792.0);
    assert_eq!(all_pages[2].height_pt, 612.0);

    // 4. Query annotations on the inserted page (index 2)
    let annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };

    assert_eq!(annots.len(), 2);
    let hl = annots
        .iter()
        .find(|a| a.kind == AnnotationKind::Highlight)
        .expect("highlight");
    let note = annots
        .iter()
        .find(|a| a.kind == AnnotationKind::Note)
        .expect("note");

    assert_eq!(note.text.as_deref(), Some("Rotated Page Note"));
    assert_eq!(note.rect, source_note.rect);
    assert_eq!(hl.rect, source_hl.rect);
}

#[test]
fn corpus_annotations_action_sample_reports_text_box_and_recreates_all_five_kinds() {
    let mut host = host();
    let file = File::open(corpus("malicious/annotations-action.pdf")).expect("open corpus");
    let (source_bytes, pages, text_boxes) = match host.prepare_source(&file, None).expect("prepare")
    {
        WorkerResponse::Source {
            bytes,
            pages,
            text_boxes,
            ..
        } => (bytes, pages, text_boxes),
        other => panic!("{other:?}"),
    };
    assert_eq!(pages, 1);
    assert_eq!(
        text_boxes.len(),
        1,
        "corpus malicious/annotations-action.pdf must report exactly 1 text box"
    );
    assert_eq!(text_boxes[0].text, "Merged Text Box");

    let model = match host
        .request(|request| WorkerRequest::PrepareTextBox {
            request,
            font: droid(),
            text: text_boxes[0].text.clone(),
        })
        .expect("prepare text box")
    {
        WorkerResponse::TextBoxPrepared { model, .. } => model,
        other => panic!("{other:?}"),
    };
    let inserted_tb = InsertedTextBox {
        page: text_boxes[0].page,
        rect: text_boxes[0].rect,
        model,
        style: text_boxes[0].style,
    };

    let (target_doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 2,
            source: source_bytes,
            text_boxes: vec![inserted_tb],
        },
    })
    .expect("insert pages");

    let annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };

    assert!(annots.iter().any(|a| a.kind == AnnotationKind::Highlight));
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::Note));
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::Ink));
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::Stamp));
    assert!(annots.iter().any(|a| a.kind == AnnotationKind::TextBox));
}

#[test]
fn annotations_completely_outside_page_bounds_are_skipped() {
    use mupdf::pdf::PdfObject;

    let mut host = host();
    let (source_doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));

    let source_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-outside-annot.pdf",
        std::process::id()
    ));
    let file = File::create(&source_path).expect("create file");
    host.save(source_doc, &file).expect("save file");
    drop(file);

    // Inject note at (5000, 5000) on a letter page (612x792)
    let modified_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-outside-annot-mod.pdf",
        std::process::id()
    ));
    {
        let doc = mupdf::pdf::PdfDocument::open(source_path.to_str().unwrap()).expect("open mupdf");
        let mut page = doc.load_pdf_page(0).expect("load page");

        let outside_note = page
            .create_annotation(mupdf::pdf::PdfAnnotationType::Text)
            .expect("create note");
        let mut note_obj = outside_note.object();
        note_obj
            .dict_put(
                "Contents",
                PdfObject::new_string("Way Outside Page Note").expect("string"),
            )
            .expect("put contents");
        let mut rect = doc.new_array().expect("array");
        for v in [5000.0, 5000.0, 5016.0, 5016.0] {
            rect.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        note_obj.dict_put("Rect", rect).expect("put rect");

        doc.save_with_options(
            modified_path.to_str().unwrap(),
            mupdf::pdf::PdfWriteOptions::default(),
        )
        .expect("save");
    }
    std::fs::remove_file(&source_path).ok();

    let file = File::open(&modified_path).expect("open file");
    let (source_bytes, pages, _) = match host.prepare_source(&file, None).expect("prepare source") {
        WorkerResponse::Source {
            bytes,
            pages,
            text_boxes,
            ..
        } => (bytes, pages, text_boxes),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&modified_path).ok();
    assert_eq!(pages, 1);

    let (target_doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 2,
            source: source_bytes,
            text_boxes: Vec::new(),
        },
    })
    .expect("insert pages");

    let annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };

    assert!(
        annots.is_empty(),
        "Outside annotations must be skipped, got: {annots:?}"
    );
}

#[test]
fn ink_with_more_than_max_strokes_is_skipped() {
    use mupdf::pdf::PdfObject;

    let mut host = host();
    let (source_doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));

    let source_path =
        std::env::temp_dir().join(format!("pdf-worker-{}-ink-strokes.pdf", std::process::id()));
    let file = File::create(&source_path).expect("create file");
    host.save(source_doc, &file).expect("save file");
    drop(file);

    let modified_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-ink-strokes-mod.pdf",
        std::process::id()
    ));
    {
        let doc = mupdf::pdf::PdfDocument::open(source_path.to_str().unwrap()).expect("open mupdf");
        let mut page = doc.load_pdf_page(0).expect("load page");

        let ink = page
            .create_annotation(mupdf::pdf::PdfAnnotationType::Ink)
            .expect("create ink");
        let mut ink_obj = ink.object();
        let mut ink_list = doc.new_array().expect("array");
        // MAX_INK_STROKES is 256. Create 260 strokes with 1 point each (total 260 points <= MAX_INK_POINTS)
        for i in 0..(MAX_INK_STROKES as usize + 4) {
            let mut stroke = doc.new_array().expect("array");
            let x = 72.0 + (i as f32) * 0.5;
            let y = 100.0;
            stroke
                .array_push(PdfObject::new_real(x).expect("real"))
                .expect("push");
            stroke
                .array_push(PdfObject::new_real(y).expect("real"))
                .expect("push");
            ink_list.array_push(stroke).expect("push stroke");
        }
        ink_obj.dict_put("InkList", ink_list).expect("put InkList");
        let mut color = doc.new_array().expect("array");
        for v in [0.0, 0.0, 0.0] {
            color
                .array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        ink_obj.dict_put("C", color).expect("put color");
        let mut rect = doc.new_array().expect("array");
        for v in [70.0, 95.0, 250.0, 105.0] {
            rect.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        ink_obj.dict_put("Rect", rect).expect("put rect");

        doc.save_with_options(
            modified_path.to_str().unwrap(),
            mupdf::pdf::PdfWriteOptions::default(),
        )
        .expect("save");
    }
    std::fs::remove_file(&source_path).ok();

    let file = File::open(&modified_path).expect("open file");
    let (source_bytes, pages, _) = match host.prepare_source(&file, None).expect("prepare source") {
        WorkerResponse::Source {
            bytes,
            pages,
            text_boxes,
            ..
        } => (bytes, pages, text_boxes),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&modified_path).ok();
    assert_eq!(pages, 1);

    let (target_doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 2,
            source: source_bytes,
            text_boxes: Vec::new(),
        },
    })
    .expect("insert pages");

    let annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };

    assert!(
        annots.is_empty(),
        "Ink with > MAX_INK_STROKES must be skipped, got: {annots:?}"
    );
}

#[test]
fn source_page_with_cropbox_preserves_annotation_geometry() {
    use mupdf::pdf::PdfObject;

    let mut host = host();
    let (source_doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));

    let source_path =
        std::env::temp_dir().join(format!("pdf-worker-{}-cropbox.pdf", std::process::id()));
    let file = File::create(&source_path).expect("create file");
    host.save(source_doc, &file).expect("save file");
    drop(file);

    let modified_path =
        std::env::temp_dir().join(format!("pdf-worker-{}-cropbox-mod.pdf", std::process::id()));
    {
        let doc = mupdf::pdf::PdfDocument::open(source_path.to_str().unwrap()).expect("open mupdf");
        let mut page = doc.load_pdf_page(0).expect("load page");
        let mut page_obj = page.object();

        // Set CropBox to [50, 60, 550, 700]
        let mut crop = doc.new_array().expect("array");
        for v in [50.0, 60.0, 550.0, 700.0] {
            crop.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        page_obj.dict_put("CropBox", crop).expect("put crop");

        // Add a note inside the cropped region
        let note = page
            .create_annotation(mupdf::pdf::PdfAnnotationType::Text)
            .expect("create note");
        let mut note_obj = note.object();
        note_obj
            .dict_put(
                "Contents",
                PdfObject::new_string("Cropped Note").expect("string"),
            )
            .expect("put contents");
        let mut rect = doc.new_array().expect("array");
        for v in [100.0, 100.0, 116.0, 116.0] {
            rect.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        note_obj.dict_put("Rect", rect).expect("put rect");

        doc.save_with_options(
            modified_path.to_str().unwrap(),
            mupdf::pdf::PdfWriteOptions::default(),
        )
        .expect("save");
    }
    std::fs::remove_file(&source_path).ok();

    // Query source note rect before merge
    let (opened_source, _) = opened(&mut host, &modified_path);
    let src_annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: opened_source,
            page_index: 0,
        })
        .expect("source annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };
    assert_eq!(src_annots.len(), 1);
    let expected_rect = src_annots[0].rect;

    let file = File::open(&modified_path).expect("open file");
    let (source_bytes, _, _) = match host.prepare_source(&file, None).expect("prepare source") {
        WorkerResponse::Source {
            bytes,
            pages,
            text_boxes,
            ..
        } => (bytes, pages, text_boxes),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&modified_path).ok();

    let (target_doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 2,
            source: source_bytes,
            text_boxes: Vec::new(),
        },
    })
    .expect("insert pages");

    let annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };

    assert_eq!(annots.len(), 1);
    let note = &annots[0];
    assert_eq!(note.text.as_deref(), Some("Cropped Note"));
    let near = |a: f32, b: f32| (a - b).abs() < 1.0;
    assert!(
        near(note.rect.x0, expected_rect.x0)
            && near(note.rect.y0, expected_rect.y0)
            && near(note.rect.x1, expected_rect.x1)
            && near(note.rect.y1, expected_rect.y1),
        "Rect mismatch: expected {expected_rect:?}, got {:?}",
        note.rect
    );
}

#[test]
fn geometric_invalid_annotations_are_skipped() {
    use mupdf::pdf::PdfObject;

    let mut host = host();
    let (source_doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));

    let source_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-geom-invalid.pdf",
        std::process::id()
    ));
    let file = File::create(&source_path).expect("create file");
    host.save(source_doc, &file).expect("save file");
    drop(file);

    let modified_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-geom-invalid-mod.pdf",
        std::process::id()
    ));
    {
        let doc = mupdf::pdf::PdfDocument::open(source_path.to_str().unwrap()).expect("open mupdf");
        let mut page = doc.load_pdf_page(0).expect("load page");

        // 1. Highlight with infinite/excessive QuadPoints (beyond MAX_PAGE_SIDE_PT)
        let hl = page
            .create_annotation(mupdf::pdf::PdfAnnotationType::Highlight)
            .expect("create hl");
        let mut hl_obj = hl.object();
        let mut qp = doc.new_array().expect("array");
        for v in [72.0, 72.0, 2_000_000.0, 72.0, 72.0, 84.0, 2_000_000.0, 84.0] {
            qp.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        hl_obj.dict_put("QuadPoints", qp).expect("put quad");
        let mut rect = doc.new_array().expect("array");
        for v in [72.0, 72.0, 200.0, 84.0] {
            rect.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        hl_obj.dict_put("Rect", rect).expect("put rect");

        // 2. Note with invalid /Rect coordinates exceeding MAX_PAGE_SIDE_PT
        let note = page
            .create_annotation(mupdf::pdf::PdfAnnotationType::Text)
            .expect("create note");
        let mut note_obj = note.object();
        note_obj
            .dict_put(
                "Contents",
                PdfObject::new_string("Invalid Rect Note").expect("string"),
            )
            .expect("put contents");
        let mut rect = doc.new_array().expect("array");
        for v in [2_000_000.0, 2_000_000.0, 2_000_016.0, 2_000_016.0] {
            rect.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        note_obj.dict_put("Rect", rect).expect("put rect");

        doc.save_with_options(
            modified_path.to_str().unwrap(),
            mupdf::pdf::PdfWriteOptions::default(),
        )
        .expect("save");
    }
    std::fs::remove_file(&source_path).ok();

    let file = File::open(&modified_path).expect("open file");
    let (source_bytes, pages, _) = match host.prepare_source(&file, None).expect("prepare source") {
        WorkerResponse::Source {
            bytes,
            pages,
            text_boxes,
            ..
        } => (bytes, pages, text_boxes),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&modified_path).ok();
    assert_eq!(pages, 1);

    let (target_doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    host.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 2,
            source: source_bytes,
            text_boxes: Vec::new(),
        },
    })
    .expect("insert pages");

    let annots = match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: target_doc,
            page_index: 2,
        })
        .expect("page annots")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };

    assert!(
        annots.is_empty(),
        "Geometrically invalid annotations must be skipped, got: {annots:?}"
    );
}

#[test]
fn source_text_boxes_beyond_limit_are_skipped() {
    use mupdf::pdf::PdfObject;

    let mut host = host();
    let (source_doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));

    let source_path =
        std::env::temp_dir().join(format!("pdf-worker-{}-tb-limit.pdf", std::process::id()));
    let file = File::create(&source_path).expect("create file");
    host.save(source_doc, &file).expect("save file");
    drop(file);

    let modified_path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-tb-limit-mod.pdf",
        std::process::id()
    ));
    {
        let tb_entry_str = "<< /V 1 /Text <FEFF0042006F0078> /Font <FEFF0041007200690061006C> \
            /Face 0 /Base /Arial /M [0 0 0 0 0 0 0 0] /G [] /S 12 /C [0 0 0] /A 0 \
            /Over false /View false >>";
        let mut doc =
            mupdf::pdf::PdfDocument::open(source_path.to_str().unwrap()).expect("open mupdf");
        let mut page = doc.load_pdf_page(0).expect("load page");
        let tb_entry = doc.new_object_from_str(tb_entry_str).expect("entry");
        let tb_entry_obj = doc.add_object(&tb_entry).expect("add entry");

        let total = MAX_SOURCE_TEXT_BOXES as usize + 5;
        for i in 0..total {
            let tb = page
                .create_annotation(mupdf::pdf::PdfAnnotationType::FreeText)
                .expect("create tb");
            let mut tb_obj = tb.object();
            tb_obj
                .dict_put("PdfReaderTextBox", tb_entry_obj.clone())
                .expect("put entry");
            let mut rect = doc.new_array().expect("array");
            let y = 10.0 + (i % 500) as f32;
            for v in [50.0, y, 150.0, y + 20.0] {
                rect.array_push(PdfObject::new_real(v).expect("real"))
                    .expect("push");
            }
            tb_obj.dict_put("Rect", rect).expect("put rect");
        }

        doc.save_with_options(
            modified_path.to_str().unwrap(),
            mupdf::pdf::PdfWriteOptions::default(),
        )
        .expect("save");
    }
    std::fs::remove_file(&source_path).ok();

    let file = File::open(&modified_path).expect("open file");
    let (_, _, text_boxes) = match host.prepare_source(&file, None).expect("prepare source") {
        WorkerResponse::Source {
            bytes,
            pages,
            text_boxes,
            ..
        } => (bytes, pages, text_boxes),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&modified_path).ok();

    assert_eq!(
        text_boxes.len(),
        MAX_SOURCE_TEXT_BOXES as usize,
        "Text boxes beyond MAX_SOURCE_TEXT_BOXES must be skipped"
    );
}
