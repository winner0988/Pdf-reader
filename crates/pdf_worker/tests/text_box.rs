//! Text boxes (B2-28, ADR 0018): a FreeText annotation whose words are set in a subset of the
//! chosen font. The font is the Droid Sans Fallback the worker carries, which has English and
//! Chinese letters. Pages are Letter size; a box is given in page space (from the top left).

use ipc_contract::types::{FontRefusal, Rect, Rgb, TextAlign, TextBoxStyle};
use mupdf::pdf::PdfDocument as MuPdf;
use pdf_worker::engine::{
    EngineError, PdfDocument, RenderedPage, TextBoxFont, TextBoxMade, TextBoxSpec, prepare_text_box,
};

#[allow(dead_code)]
mod common;

const BOX: Rect = Rect {
    x0: 72.0,
    y0: 72.0,
    x1: 372.0,
    y1: 132.0,
};
const MIXED: &str = "Hello 你好，世界";
const BLUE: Rgb = Rgb {
    r: 26,
    g: 51,
    b: 153,
};

fn droid() -> &'static [u8] {
    mupdf_fonts_droid::find_by_name("Droid Sans Fallback", false, false)
        .expect("the bundled font")
        .data
}

fn font(bytes: &[u8]) -> TextBoxFont<'_> {
    TextBoxFont {
        bytes,
        face: 0,
        name: "Droid Sans Fallback",
    }
}

fn spec(text: &str) -> TextBoxSpec<'_> {
    TextBoxSpec {
        text,
        size: 24.0,
        color: BLUE,
        align: TextAlign::Left,
    }
}

fn blank() -> PdfDocument {
    PdfDocument::from_bytes(&common::one_page("")).expect("a page")
}

fn saved(doc: &mut PdfDocument) -> Vec<u8> {
    let mut out = Vec::new();
    doc.save(&mut out).expect("save");
    out
}

fn render(bytes: &[u8]) -> RenderedPage {
    PdfDocument::from_bytes(bytes)
        .expect("open")
        .render(0, 1.0, 0)
        .expect("render")
}

/// The pixels of `page` inside `area` (left, top, right, bottom) for which `wanted` says yes.
fn pixels(
    page: &RenderedPage,
    area: [usize; 4],
    wanted: impl Fn([i32; 3]) -> bool,
) -> Vec<(usize, usize)> {
    let [x0, y0, x1, y1] = area;
    let mut found = Vec::new();
    for y in y0..y1.min(page.height as usize) {
        for x in x0..x1.min(page.width as usize) {
            let at = (y * page.width as usize + x) * 4;
            let [r, g, b] = [page.rgba[at], page.rgba[at + 1], page.rgba[at + 2]].map(i32::from);
            if wanted([r, g, b]) {
                found.push((x, y));
            }
        }
    }
    found
}

/// Dark pixels, whatever their colour.
fn ink(page: &RenderedPage, area: [usize; 4]) -> Vec<(usize, usize)> {
    pixels(page, area, |[r, g, b]| r + g + b < 3 * 160)
}

fn whole(page: &RenderedPage) -> [usize; 4] {
    [0, 0, page.width as usize, page.height as usize]
}

const IN_BOX: [usize; 4] = [72, 72, 372, 132];

/// The box that contains `points`: left, top, right, bottom.
fn bounds(points: &[(usize, usize)]) -> [usize; 4] {
    let xs = points.iter().map(|p| p.0);
    let ys = points.iter().map(|p| p.1);
    [
        xs.clone().min().expect("ink"),
        ys.clone().min().expect("ink"),
        xs.max().expect("ink") + 1,
        ys.max().expect("ink") + 1,
    ]
}

#[test]
fn a_text_box_is_drawn_from_a_subset_and_survives_a_save() {
    let mut doc = blank();
    let made = doc
        .add_text_box(0, BOX, &spec(MIXED), &font(droid()))
        .expect("a text box");
    assert_eq!(
        made,
        TextBoxMade {
            view_only: false,
            overflow: false
        }
    );
    let bytes = saved(&mut doc);
    // The font file is 3.5 MB; what the file holds of it is the glyphs the box uses.
    assert!(bytes.len() < 20_000, "{} bytes", bytes.len());

    let page = render(&bytes);
    let inside = ink(&page, IN_BOX);
    assert!(inside.len() > 100, "the box is drawn: {}", inside.len());
    assert_eq!(
        ink(&page, whole(&page)).len(),
        inside.len(),
        "and nothing else is"
    );
    let [left, top, right, bottom] = bounds(&inside);
    assert!(
        left >= 72 && top >= 72 && right <= 372 && bottom <= 132,
        "{left} {top} {right} {bottom}"
    );

    // What another reader finds: a FreeText annotation that says its words, in a subset font.
    let reopened = MuPdf::from_bytes(&bytes).expect("open");
    let page = reopened.load_pdf_page(0).expect("page");
    let annotation = page.annotations().next().expect("an annotation");
    let dict = annotation.object();
    assert_eq!(
        dict.get_dict("Subtype")
            .unwrap()
            .unwrap()
            .as_name()
            .unwrap(),
        b"FreeText"
    );
    assert_eq!(annotation.contents().expect("contents"), Some(MIXED));
    let fonts = dict
        .get_dict("AP")
        .unwrap()
        .unwrap()
        .get_dict("N")
        .unwrap()
        .unwrap()
        .get_dict("Resources")
        .unwrap()
        .unwrap()
        .get_dict("Font")
        .unwrap()
        .unwrap();
    assert_eq!(fonts.dict_len().unwrap(), 1);
    let type0 = fonts.get_dict_val(0).unwrap().unwrap();
    let base = type0
        .get_dict("BaseFont")
        .unwrap()
        .unwrap()
        .as_name()
        .unwrap();
    assert_eq!(base[6], b'+', "{}", String::from_utf8_lossy(&base));
    assert!(base[..6].iter().all(u8::is_ascii_uppercase));
    assert!(
        type0.get_dict("ToUnicode").unwrap().is_some(),
        "other readers can search and copy it"
    );
    // The user's choices are kept, to build the box again.
    assert!(dict.get_dict("PdfReaderTextBox").unwrap().is_some());
}

#[test]
fn the_colour_and_the_size_are_the_ones_asked_for() {
    let blue =
        |page: &RenderedPage| pixels(page, whole(page), |[r, g, b]| b > r + 40 && b > g + 20).len();
    let height_of = |size: f32| {
        let mut doc = blank();
        let text = TextBoxSpec {
            size,
            ..spec("HHH")
        };
        doc.add_text_box(0, Rect { y1: 400.0, ..BOX }, &text, &font(droid()))
            .expect("box");
        let page = render(&saved(&mut doc));
        assert!(blue(&page) > 50, "the colour: {}", blue(&page));
        let [_, top, _, bottom] = bounds(&ink(&page, whole(&page)));
        bottom - top
    };
    let (small, large) = (height_of(12.0), height_of(48.0));
    assert!(large > small * 3, "{small} and {large} pixels tall");
}

#[test]
fn text_is_set_at_the_left_in_the_middle_or_at_the_right_of_the_box() {
    let mut spans = Vec::new();
    for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
        let mut doc = blank();
        let text = TextBoxSpec {
            align,
            ..spec("Hello")
        };
        doc.add_text_box(0, BOX, &text, &font(droid()))
            .expect("box");
        let [left, _, right, _] = bounds(&ink(&render(&saved(&mut doc)), IN_BOX));
        spans.push((left, right));
    }
    let [left, middle, right] = [spans[0], spans[1], spans[2]];
    assert!(left.0 <= 72 + 6, "{left:?}");
    assert!(right.1 + 6 >= 372, "{right:?}");
    let centre = (middle.0 + middle.1) as f32 / 2.0;
    assert!((centre - 222.0).abs() <= 6.0, "{middle:?}");
    // The same words, so the same width in all three.
    let widths = [left, middle, right].map(|(first, last)| last as i32 - first as i32);
    assert!(
        widths.iter().all(|width| (width - widths[0]).abs() <= 2),
        "{widths:?}"
    );
}

#[test]
fn moving_a_text_box_by_its_rect_alone_moves_it_and_nothing_else_changes() {
    let mut doc = blank();
    doc.add_text_box(0, BOX, &spec(MIXED), &font(droid()))
        .expect("box");
    let bytes = saved(&mut doc);
    let before = render(&bytes);
    let first = ink(&before, whole(&before));
    let moved = MuPdf::from_bytes(&bytes).expect("open");
    let page = moved.load_pdf_page(0).expect("page");
    let mut annotation = page
        .object()
        .get_dict("Annots")
        .unwrap()
        .unwrap()
        .get_array(0)
        .unwrap()
        .unwrap();
    let rect = moved
        .new_object_from_str("[172 460 472 520]")
        .expect("rect");
    annotation.dict_put("Rect", rect).expect("rect");
    let mut again = Vec::new();
    moved.write_to(&mut again).expect("write");
    let page = render(&again);
    assert!(ink(&page, [172, 272, 472, 332]).len() > 100);
    assert!(ink(&page, IN_BOX).is_empty());
    // The same pixels, a hundred points to the right and two hundred down.
    let after = ink(&page, whole(&page));
    assert_eq!(after.len(), first.len());
}

#[test]
fn a_box_on_a_turned_page_is_set_upright_where_it_was_asked() {
    let straight = {
        let mut doc = blank();
        doc.add_text_box(0, BOX, &spec(MIXED), &font(droid()))
            .expect("box");
        let page = render(&saved(&mut doc));
        ink(&page, IN_BOX)
    };
    for degrees in [90, 180, 270] {
        let mut doc = blank();
        doc.rotate_pages(&[0], degrees).expect("turn");
        doc.add_text_box(0, BOX, &spec(MIXED), &font(droid()))
            .expect("box");
        let page = render(&saved(&mut doc));
        let inside = ink(&page, IN_BOX);
        assert_eq!(
            ink(&page, whole(&page)).len(),
            inside.len(),
            "{degrees}: nothing outside the box"
        );
        // The same words in the same place: the same ink, give or take what a turn does to the
        // anti-aliasing.
        assert_eq!(bounds(&inside), bounds(&straight), "{degrees}");
        let (a, b) = (inside.len() as f32, straight.len() as f32);
        assert!((a - b).abs() / b < 0.1, "{degrees}: {a} against {b}");
    }
}

fn blue(page: &RenderedPage) -> usize {
    pixels(page, whole(page), |[r, g, b]| b > r + 40 && b > g + 20).len()
}

/// The `BaseFont` of every font in the appearance of the first annotation of the first page.
fn appearance_fonts(bytes: &[u8]) -> Vec<Vec<u8>> {
    let doc = MuPdf::from_bytes(bytes).expect("open");
    let page = doc.load_pdf_page(0).expect("page");
    let annotation = page.annotations().next().expect("an annotation");
    let Some(ap) = annotation.object().get_dict("AP").unwrap() else {
        return Vec::new();
    };
    let Some(n) = ap.get_dict("N").unwrap() else {
        return Vec::new();
    };
    let Some(fonts) = n
        .get_dict("Resources")
        .unwrap()
        .and_then(|r| r.get_dict("Font").unwrap())
    else {
        return Vec::new();
    };
    (0..fonts.dict_len().unwrap() as i32)
        .filter_map(|at| fonts.get_dict_val(at).unwrap())
        .filter_map(|font| font.get_dict("BaseFont").unwrap())
        .map(|base| base.as_name().unwrap())
        .collect()
}

fn is_subset(base: &[u8]) -> bool {
    base.len() > 7 && base[6] == b'+' && base[..6].iter().all(u8::is_ascii_uppercase)
}

#[test]
fn mupdf_draws_the_appearance_again_when_it_is_asked_to_change_the_annotation() {
    // The reason a text box is never changed through MuPDF (the module's rule): it makes a new
    // appearance from `/DA`, in Helvetica or a substitute, and the subset the user chose is no
    // longer in it. If MuPDF stops doing that, this fails, and the rule can be looked at again.
    let mut doc = blank();
    doc.add_text_box(0, BOX, &spec(MIXED), &font(droid()))
        .expect("box");
    let first = saved(&mut doc);
    assert!(appearance_fonts(&first).iter().all(|base| is_subset(base)));
    for change in ["contents", "rect"] {
        let doc = MuPdf::from_bytes(&first).expect("open");
        {
            let page = doc.load_pdf_page(0).expect("page");
            let mut annotation = page.annotations().next().expect("an annotation");
            if change == "contents" {
                annotation.set_contents("Hello 你好").expect("contents");
            } else {
                let mut rect = annotation.rect().expect("rect");
                rect.y0 += 1.0;
                rect.y1 += 1.0;
                annotation.set_rect(rect).expect("rect");
            }
            annotation.update().expect("update");
        }
        let mut second = Vec::new();
        doc.write_to(&mut second).expect("write");
        let fonts = appearance_fonts(&second);
        assert!(
            !fonts.iter().any(|base| is_subset(base)),
            "{change}: {fonts:?}"
        );
    }
}

#[test]
fn the_selection_and_removal_of_annotations_see_a_text_box_and_removal_takes_its_font_with_it() {
    use ipc_contract::types::AnnotationKind;
    let mut doc = blank();
    let empty = saved(&mut blank()).len();
    doc.add_text_box(0, BOX, &spec(MIXED), &font(droid()))
        .expect("box");
    let mut app = PdfDocument::from_bytes(&saved(&mut doc)).expect("open");
    let listed = app.page_annotations(0).expect("annotations");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind, AnnotationKind::TextBox);
    assert_eq!(listed[0].text.as_deref(), Some(MIXED));
    let rect = listed[0].rect;
    assert_eq!(
        [rect.x0, rect.y0, rect.x1, rect.y1],
        [72.0, 72.0, 372.0, 132.0]
    );
    let kept = saved(&mut app);
    assert!(kept.len() < 20_000, "{}", kept.len());
    assert!(
        blue(&render(&kept)) > 100,
        "the app's own save keeps the appearance"
    );
    app.delete_annotation(0, listed[0].id).expect("delete");
    let gone = saved(&mut app);
    assert!(gone.len() < empty + 300, "{} against {empty}", gone.len());
}

/// A copy of the font with its table directory changed by `change`, in order again afterwards.
fn changed(change: impl Fn(&mut Vec<[u8; 16]>, &mut Vec<u8>)) -> Vec<u8> {
    let mut data = droid().to_vec();
    let count = u16::from_be_bytes([data[4], data[5]]) as usize;
    let mut records: Vec<[u8; 16]> = (0..count)
        .map(|at| {
            data[12 + at * 16..28 + at * 16]
                .try_into()
                .expect("a record")
        })
        .collect();
    change(&mut records, &mut data);
    records.sort_by_key(|record| [record[0], record[1], record[2], record[3]]);
    for (at, record) in records.iter().enumerate() {
        data[12 + at * 16..28 + at * 16].copy_from_slice(record);
    }
    data
}

/// The font with its table `from` called `to`.
fn renamed(from: &[u8; 4], to: &[u8; 4]) -> Vec<u8> {
    changed(|records, _| {
        let record = records
            .iter_mut()
            .find(|record| &record[..4] == from)
            .expect("the table");
        record[..4].copy_from_slice(to);
    })
}

/// The font with the licence flags (`fsType`) of its OS/2 table set to `value`.
fn with_fs_type(value: u16) -> Vec<u8> {
    changed(|records, data| {
        let record = records
            .iter()
            .find(|record| &record[..4] == b"OS/2")
            .expect("OS/2");
        let at = u32::from_be_bytes(record[8..12].try_into().unwrap()) as usize + 8;
        data[at..at + 2].copy_from_slice(&value.to_be_bytes());
    })
}

fn refusal(result: Result<TextBoxMade, EngineError>) -> FontRefusal {
    match result {
        Err(EngineError::Font(refusal)) => refusal,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_font_that_cannot_be_used_makes_no_text_box_and_leaves_the_document_as_it_was() {
    let before = saved(&mut blank());
    let cases = [
        (
            b"not a font at all".to_vec(),
            "words",
            FontRefusal::NotAFont,
        ),
        (renamed(b"post", b"fvar"), "words", FontRefusal::Variable),
        (renamed(b"post", b"COLR"), "words", FontRefusal::Colour),
        (renamed(b"glyf", b"glyx"), "words", FontRefusal::NotTrueType),
        (with_fs_type(0x0002), "words", FontRefusal::NotEmbeddable),
        (with_fs_type(0x0100), "words", FontRefusal::NotEmbeddable),
        (with_fs_type(0x0200), "words", FontRefusal::NotEmbeddable),
        (
            droid().to_vec(),
            "words \u{e000}",
            FontRefusal::MissingGlyphs,
        ),
    ];
    for (bytes, text, expected) in cases {
        let mut doc = blank();
        assert_eq!(
            refusal(doc.add_text_box(0, BOX, &spec(text), &font(&bytes))),
            expected
        );
        assert!(
            saved(&mut doc) == before,
            "{expected:?}: something was put in the document"
        );
    }
    // A face that the file does not have is not a font either.
    let mut doc = blank();
    let third = TextBoxFont {
        face: 2,
        ..font(droid())
    };
    assert_eq!(
        refusal(doc.add_text_box(0, BOX, &spec("words"), &third)),
        FontRefusal::NotAFont
    );
}

#[test]
fn a_font_for_viewing_and_printing_only_is_used_and_says_so_and_one_that_may_be_edited_is_not() {
    for (fs_type, view_only) in [(0x0000, false), (0x0008, false), (0x0004, true)] {
        let bytes = with_fs_type(fs_type);
        let made = blank()
            .add_text_box(0, BOX, &spec("words"), &font(&bytes))
            .expect("box");
        assert_eq!(made.view_only, view_only, "{fs_type:#06x}");
    }
}

#[test]
fn a_box_too_small_for_its_words_says_so() {
    let long = "Many words that do not fit in a box that is one line tall. ".repeat(4);
    let small = Rect { y1: 100.0, ..BOX };
    let made = blank()
        .add_text_box(0, small, &spec(&long), &font(droid()))
        .expect("box");
    assert!(made.overflow);
    let made = blank()
        .add_text_box(0, BOX, &spec("Hello"), &font(droid()))
        .expect("box");
    assert!(!made.overflow);
}

#[test]
fn a_text_box_that_asks_for_something_unreasonable_is_refused() {
    let mut doc = blank();
    let before = saved(&mut doc);
    let huge = "x".repeat(5_000);
    let bad: Vec<(TextBoxSpec, Rect)> = vec![
        (spec(""), BOX),
        (spec(" \n  "), BOX),
        (spec(&huge), BOX),
        (
            TextBoxSpec {
                size: 3.0,
                ..spec("x")
            },
            BOX,
        ),
        (
            TextBoxSpec {
                size: 300.0,
                ..spec("x")
            },
            BOX,
        ),
        (
            TextBoxSpec {
                size: f32::NAN,
                ..spec("x")
            },
            BOX,
        ),
        (spec("x"), Rect { x1: 75.0, ..BOX }),
        (
            spec("x"),
            Rect {
                y1: f32::INFINITY,
                ..BOX
            },
        ),
        (
            spec("x"),
            Rect {
                x0: 372.0,
                x1: 72.0,
                ..BOX
            },
        ),
    ];
    for (text, rect) in &bad {
        let result = doc.add_text_box(0, *rect, text, &font(droid()));
        assert!(
            matches!(result, Err(EngineError::InvalidEdit(_))),
            "{text:?} {rect:?}: {result:?}"
        );
    }
    assert!(
        doc.add_text_box(1, BOX, &spec("x"), &font(droid()))
            .is_err(),
        "a page that is not there"
    );
    // Scripts whose letters change by their neighbours, and the direction controls, are not set
    // wrongly: they are refused as what they are.
    for text in [
        "\u{628}\u{644}",
        "\u{5d0}\u{5d1}",
        "\u{915}",
        "\u{e01}",
        "e\u{301}",
        "words \u{202e}drow",
        "a\u{200d}b",
    ] {
        let result = doc.add_text_box(0, BOX, &spec(text), &font(droid()));
        assert!(
            matches!(result, Err(EngineError::UnsupportedText)),
            "{text:?}: {result:?}"
        );
    }
    assert!(
        saved(&mut doc) == before,
        "something was put in the document"
    );
}

fn style(size: f32) -> TextBoxStyle {
    TextBoxStyle {
        size,
        color: BLUE,
        align: TextAlign::Left,
    }
}

#[test]
fn a_model_is_a_few_kilobytes_and_is_set_again_in_other_documents_without_the_font() {
    let model = prepare_text_box(&font(droid()), MIXED).expect("a model");
    // The font file is 3.5 MB; the model holds the glyphs of the words.
    assert!(model.subset.len() < 20_000, "{} bytes", model.subset.len());
    assert!(!model.view_only);
    assert_eq!(model.text, MIXED);
    assert!(model.base.as_bytes()[6] == b'+' && model.glyphs.len() == 10);

    // The same model, at two sizes in two documents: no font file is needed to set it.
    let mut spans = Vec::new();
    for size in [14.0, 28.0] {
        let mut doc = blank();
        let made = doc
            .place_text_box(0, BOX, &model, &style(size))
            .expect("placed");
        assert!(!made.overflow);
        let page = render(&saved(&mut doc));
        let [left, top, right, bottom] = bounds(&ink(&page, IN_BOX));
        spans.push((right - left, bottom - top));
    }
    assert!(spans[1].0 > spans[0].0 * 3 / 2, "{spans:?}");
    assert!(spans[1].1 > spans[0].1 * 3 / 2, "{spans:?}");

    // Setting the model is setting a box: the same objects in the same order, so the same
    // numbers, which is what undo needs of an edit that is made again.
    let (mut one, mut two) = (blank(), blank());
    one.add_text_box(0, BOX, &spec(MIXED), &font(droid()))
        .expect("box");
    two.place_text_box(0, BOX, &model, &style(24.0))
        .expect("box");
    let ids = |doc: &PdfDocument| {
        doc.page_annotations(0)
            .expect("list")
            .iter()
            .map(|a| a.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&one), ids(&two));
}

#[test]
fn the_model_and_the_style_are_read_back_from_the_saved_file() {
    let model = prepare_text_box(&font(droid()), "Hello\n你好，世界").expect("a model");
    let wanted = TextBoxStyle {
        size: 17.5,
        color: Rgb {
            r: 200,
            g: 0,
            b: 40,
        },
        align: TextAlign::Right,
    };
    let mut doc = blank();
    let made = doc.place_text_box(0, BOX, &model, &wanted).expect("placed");
    let reopened = PdfDocument::from_bytes(&saved(&mut doc)).expect("open");
    let id = reopened.page_annotations(0).expect("list")[0].id;
    let (read, style, overflow) = reopened.text_box(0, id).expect("a text box");
    assert_eq!(read.text, model.text);
    assert_eq!(
        (read.font.as_str(), read.face, read.view_only),
        (model.font.as_str(), model.face, model.view_only)
    );
    assert_eq!(read.base, model.base);
    assert_eq!(read.glyphs, model.glyphs);
    assert_eq!(
        (
            read.ascent,
            read.descent,
            read.cap_height,
            read.step,
            read.bbox
        ),
        (
            model.ascent,
            model.descent,
            model.cap_height,
            model.step,
            model.bbox
        )
    );
    assert_eq!(read.subset.len(), model.subset.len().max(read.subset.len()));
    assert_eq!(style, wanted);
    assert_eq!(overflow, made.overflow);
    // What was read sets the box again, with another colour, and nothing else about it changes.
    let recoloured = TextBoxStyle {
        color: BLUE,
        ..style
    };
    let mut again = blank();
    again
        .place_text_box(0, BOX, &read, &recoloured)
        .expect("again");
    assert!(blue(&render(&saved(&mut again))) > 50);
}

#[test]
fn a_model_that_was_tampered_with_makes_no_text_box() {
    let model = prepare_text_box(&font(droid()), "Hello 你好").expect("a model");
    let mut doc = blank();
    let before = saved(&mut doc);
    let mut broken: Vec<(&str, ipc_contract::worker::TextBoxModel)> = Vec::new();
    // A character without a glyph, a glyph past the end of the subset, a subset that is no font.
    let mut missing = model.clone();
    missing.glyphs.pop();
    broken.push(("a missing glyph", missing));
    let mut past = model.clone();
    past.glyphs[0].cid = 4_000;
    broken.push(("a glyph past the end", past));
    let mut garbage = model.clone();
    garbage.subset = [&[0u8, 1, 0, 0][..], &[7u8; 64][..]].concat();
    broken.push(("a subset that is no font", garbage));
    let mut named = model.clone();
    named.base = "Arial".to_owned();
    broken.push(("a name that is no subset's", named));
    let mut scripts = model.clone();
    scripts.text = "\u{628}".to_owned();
    broken.push(("text that needs shaping", scripts));
    for (what, model) in broken {
        let result = doc.place_text_box(0, BOX, &model, &style(12.0));
        assert!(result.is_err(), "{what}");
    }
    assert!(
        saved(&mut doc) == before,
        "something was put in the document"
    );
}

#[test]
fn only_a_free_text_with_the_apps_entry_is_a_text_box_and_the_entry_says_what_the_box_needs() {
    use ipc_contract::types::AnnotationKind;
    let mut doc = blank();
    let small = Rect {
        x1: 100.0,
        y1: 82.0,
        ..BOX
    };
    let spec_long = spec("one two three four five six seven eight nine ten");
    doc.add_text_box(0, small, &spec_long, &font(droid()))
        .expect("box");
    let bytes = saved(&mut doc);
    let doc = MuPdf::from_bytes(&bytes).expect("open");
    let page = doc.load_pdf_page(0).expect("page");
    let annotation = page.annotations().next().expect("an annotation");
    let entry = annotation
        .object()
        .get_dict("PdfReaderTextBox")
        .unwrap()
        .expect("the entry");
    let flag = |key: &str| entry.get_dict(key).unwrap().and_then(|v| v.as_bool().ok());
    assert_eq!(flag("Over"), Some(true));
    assert_eq!(flag("View"), Some(false));

    // The same annotation without the entry, as another program would have made it: not ours.
    let app = PdfDocument::from_bytes(&bytes).expect("open");
    assert_eq!(
        app.page_annotations(0).expect("list")[0].kind,
        AnnotationKind::TextBox
    );
    let stripped = {
        let raw = MuPdf::from_bytes(&bytes).expect("open");
        let page = raw.load_pdf_page(0).expect("page");
        let annotation = page.annotations().next().expect("an annotation");
        annotation
            .object()
            .dict_delete("PdfReaderTextBox")
            .expect("delete");
        let mut out = Vec::new();
        raw.write_to(&mut out).expect("write");
        out
    };
    let app = PdfDocument::from_bytes(&stripped).expect("open");
    let listed = app.page_annotations(0).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind, AnnotationKind::Other);
    assert_eq!(listed[0].text, None);
}

#[test]
fn english_traditional_and_simplified_chinese_and_japanese_are_each_drawn_inside_the_box() {
    // Characters that only one of the three CJK languages writes this way are in each line.
    let lines = [
        ("english", "The quick brown fox"),
        ("traditional", "繁體中文的文字方塊，體驗學習"),
        ("simplified", "简体中文的文字方框，体验学习"),
        ("japanese", "日本語のテキスト、ひらがなとカタカナ"),
    ];
    let mut sizes = Vec::new();
    for (language, text) in lines {
        let mut doc = blank();
        let made = doc
            .add_text_box(0, BOX, &spec(text), &font(droid()))
            .unwrap_or_else(|error| panic!("{language}: {error}"));
        assert!(!made.overflow, "{language}");
        let bytes = saved(&mut doc);
        sizes.push(bytes.len());
        let page = render(&bytes);
        let inside = ink(&page, IN_BOX);
        assert!(inside.len() > 100, "{language}: {}", inside.len());
        assert_eq!(ink(&page, whole(&page)).len(), inside.len(), "{language}");
        // The words are in the file for other readers, and the font is a few kilobytes.
        let reopened = MuPdf::from_bytes(&bytes).expect("open");
        let annotation = reopened
            .load_pdf_page(0)
            .expect("page")
            .annotations()
            .next()
            .expect("an annotation");
        assert_eq!(annotation.contents().expect("contents"), Some(text));
        assert!(bytes.len() < 40_000, "{language}: {} bytes", bytes.len());
    }
}

/// The page as the app opens a file it saved, and the id of its one text box.
fn reopened(doc: &mut PdfDocument) -> (PdfDocument, ipc_contract::types::AnnotationId) {
    let app = PdfDocument::from_bytes(&saved(doc)).expect("open");
    let id = app.page_annotations(0).expect("list")[0].id;
    (app, id)
}

#[test]
fn a_text_box_moved_through_the_annotation_edit_keeps_its_colour_and_its_font() {
    let mut doc = blank();
    doc.add_text_box(0, BOX, &spec(MIXED), &font(droid()))
        .expect("box");
    let (mut app, id) = reopened(&mut doc);
    let before = render(&saved(&mut app));
    let moved = Rect {
        x0: 172.0,
        y0: 272.0,
        x1: 472.0,
        y1: 332.0,
    };
    app.set_annotation_rect(0, id, moved).expect("moved");
    let bytes = saved(&mut app);
    let page = render(&bytes);
    assert!(
        ink(&page, IN_BOX).is_empty(),
        "nothing is left where it was"
    );
    assert_eq!(
        ink(&page, [172, 272, 472, 332]).len(),
        ink(&before, IN_BOX).len(),
        "the same pixels, a hundred points to the right and two hundred down"
    );
    assert!(blue(&page) > 100, "still blue, not black");
    assert!(appearance_fonts(&bytes).iter().all(|base| is_subset(base)));
    let listed = PdfDocument::from_bytes(&bytes)
        .expect("open")
        .page_annotations(0)
        .expect("list");
    assert_eq!(listed[0].id, id, "the same annotation");
    let rect = listed[0].rect;
    assert_eq!(
        [rect.x0, rect.y0, rect.x1, rect.y1],
        [172.0, 272.0, 472.0, 332.0]
    );
}

#[test]
fn a_text_box_resized_sets_its_words_again_from_the_file_without_the_font() {
    let mut doc = blank();
    let long = "one two three four five six seven eight nine ten eleven twelve";
    doc.add_text_box(0, BOX, &spec(long), &font(droid()))
        .expect("box");
    let (mut app, id) = reopened(&mut doc);
    let tall = |app: &mut PdfDocument| {
        let page = render(&saved(app));
        let [_, top, _, bottom] = bounds(&ink(&page, whole(&page)));
        bottom - top
    };
    let narrow = tall(&mut app);
    // A wider box: the lines break differently, so there are fewer of them.
    let wide = Rect {
        x1: 572.0,
        y1: 200.0,
        ..BOX
    };
    app.set_annotation_rect(0, id, wide).expect("resized");
    let wider = tall(&mut app);
    assert!(wider < narrow, "{wider} against {narrow} pixels tall");
    let bytes = saved(&mut app);
    assert!(blue(&render(&bytes)) > 100, "still blue");
    // What the file says of the box is what it said, and it is still the same annotation.
    let again = PdfDocument::from_bytes(&bytes).expect("open");
    assert_eq!(again.page_annotations(0).expect("list")[0].id, id);
    let (model, style, overflow) = again.text_box(0, id).expect("a text box");
    assert_eq!(model.text, long);
    assert_eq!((style.size, style.color), (24.0, BLUE));
    assert!(!overflow);
    let rect = again.page_annotations(0).expect("list")[0].rect;
    assert_eq!([rect.x0, rect.x1, rect.y1], [72.0, 572.0, 200.0]);
}

#[test]
fn a_text_box_gets_another_style_and_other_words_and_the_old_font_goes_with_a_full_save() {
    let mut doc = blank();
    doc.add_text_box(0, BOX, &spec("Hello 你好，世界"), &font(droid()))
        .expect("box");
    let (mut app, id) = reopened(&mut doc);
    let first = saved(&mut app);

    // Another size, colour and alignment, with the words it has.
    let red = TextBoxStyle {
        size: 36.0,
        color: Rgb {
            r: 200,
            g: 20,
            b: 20,
        },
        align: TextAlign::Right,
    };
    let made = app.set_text_box(0, id, None, &red).expect("restyled");
    assert!(!made.overflow || made.overflow);
    let page = render(&saved(&mut app));
    assert!(pixels(&page, whole(&page), |[r, g, b]| r > g + 80 && r > b + 80).len() > 100);
    assert_eq!(blue(&page), 0, "no longer blue");
    let (model, style, _) = app.text_box(0, id).expect("a text box");
    assert_eq!((model.text.as_str(), style), ("Hello 你好，世界", red));

    // Other words, prepared from the font: the text of the box and its glyphs are the new ones.
    let other = prepare_text_box(&font(droid()), "Goodbye 再見，朋友").expect("a model");
    app.set_text_box(0, id, Some(&other), &style)
        .expect("new words");
    let listed = app.page_annotations(0).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].text.as_deref(), Some("Goodbye 再見，朋友"));
    let last = saved(&mut app);
    // The first subset is not in the file any more: it is about the size of the first save.
    assert!(
        last.len() < first.len() + 4_000,
        "{} against {}",
        last.len(),
        first.len()
    );
    assert!(appearance_fonts(&last).iter().all(|base| is_subset(base)));
}

#[test]
fn only_a_text_box_of_the_app_is_set_again_and_it_needs_a_rectangle_a_box_can_have() {
    let mut doc = blank();
    doc.add_text_box(0, BOX, &spec("Hello"), &font(droid()))
        .expect("box");
    let (mut app, id) = reopened(&mut doc);
    let before = saved(&mut app);
    // Not a box and not a rectangle.
    assert!(
        app.set_annotation_rect(0, ipc_contract::types::AnnotationId(9_999), BOX)
            .is_err()
    );
    for rect in [
        Rect { x1: 75.0, ..BOX },
        Rect {
            y1: f32::NAN,
            ..BOX
        },
        Rect {
            x0: 372.0,
            x1: 72.0,
            ..BOX
        },
    ] {
        assert!(app.set_annotation_rect(0, id, rect).is_err(), "{rect:?}");
    }
    let other = prepare_text_box(&font(droid()), "Hello").expect("a model");
    let wrong_style = TextBoxStyle {
        size: 1_000.0,
        ..style(12.0)
    };
    assert!(app.set_text_box(0, id, Some(&other), &wrong_style).is_err());
    assert!(
        saved(&mut app) == before,
        "something was changed by what was refused"
    );
}
