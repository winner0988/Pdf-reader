//! Text boxes (B2-28, ADR 0018): a FreeText annotation whose words are set in a subset of the
//! chosen font. The font is the Droid Sans Fallback the worker carries, which has English and
//! Chinese letters. Pages are Letter size; a box is given in page space (from the top left).

use ipc_contract::types::{FontRefusal, Rect, Rgb, TextAlign};
use mupdf::pdf::PdfDocument as MuPdf;
use pdf_worker::engine::{
    EngineError, PdfDocument, RenderedPage, TextBoxFont, TextBoxMade, TextBoxSpec,
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
    assert_eq!(listed[0].kind, AnnotationKind::Other);
    let rect = listed[0].rect;
    assert_eq!(
        [rect.x0, rect.y0, rect.x1, rect.y1],
        [72.0, 72.0, 372.0, 132.0]
    );
    // Moving goes through MuPDF's setters, which would draw it black: it is refused until B2-29.
    assert!(app.set_annotation_rect(0, listed[0].id, rect).is_err());
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
        // Scripts whose letters change by their neighbours, and the direction controls.
        (spec("\u{628}\u{644}"), BOX),
        (spec("\u{5d0}\u{5d1}"), BOX),
        (spec("e\u{301}"), BOX),
        (spec("words \u{202e}drow"), BOX),
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
    assert!(
        saved(&mut doc) == before,
        "something was put in the document"
    );
}
