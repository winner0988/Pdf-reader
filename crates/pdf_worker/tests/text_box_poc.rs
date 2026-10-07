//! POC for ADR 0018 (proposed, #185): a text box is a FreeText annotation whose appearance the worker
//! makes itself, in a subset of an installed font. Nothing here is the app's code: it shows what
//! the pieces cost and where MuPDF gets in the way. The ADR's list of holes is the tests that say
//! "is not" or "replaces".

#![allow(unsafe_code)] // only `mupdf_subset`, which measures MuPDF's own subsetter

use std::collections::BTreeMap;

use mupdf::pdf::{PdfDocument as MuPdf, PdfObject};
use mupdf::{Buffer, Font};
use pdf_worker::engine::{PdfDocument, RenderedPage};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};
use subsetter::{GlyphRemapper, subset};

#[allow(dead_code)]
mod common;

const PAGE_HEIGHT: f32 = 792.0;
/// A box in PDF space: 300 x 60 points, 72 from the left, 72 from the top.
const BOX: [f32; 4] = [72.0, PAGE_HEIGHT - 132.0, 372.0, PAGE_HEIGHT - 72.0];
const MIXED: &str = "Hello 你好，世界";

/// An installed font as the main process would hand it over: its bytes, and which face.
fn droid() -> &'static [u8] {
    mupdf_fonts_droid::find_by_name("Droid Sans Fallback", false, false)
        .expect("the bundled font")
        .data
}

/// A character, its glyph in the font and the advance it takes, in em.
#[derive(Clone, Copy)]
struct Glyph {
    ch: char,
    id: u16,
    advance: f32,
}

fn shape(font: &FontRef, text: &str) -> Vec<Glyph> {
    let charmap = font.charmap();
    let metrics = font.glyph_metrics(
        skrifa::instance::Size::unscaled(),
        skrifa::instance::LocationRef::default(),
    );
    let per_em = f32::from(font.head().expect("head").units_per_em());
    text.chars()
        .map(|ch| {
            let id = charmap.map(ch).map_or(0, |glyph| glyph.to_u32() as u16);
            let advance = metrics
                .advance_width(skrifa::GlyphId::new(u32::from(id)))
                .unwrap_or(0.0)
                / per_em;
            Glyph { ch, id, advance }
        })
        .collect()
}

/// Breaks `glyphs` into lines no wider than `width` points at `size`: at a space when there is
/// one in the line, else before the glyph that does not fit (Chinese has no spaces).
fn wrap(glyphs: &[Glyph], size: f32, width: f32) -> Vec<Vec<Glyph>> {
    let mut lines: Vec<Vec<Glyph>> = vec![Vec::new()];
    let mut used = 0.0;
    for &glyph in glyphs {
        if glyph.ch == '\n' {
            lines.push(Vec::new());
            used = 0.0;
            continue;
        }
        let step = glyph.advance * size;
        if used + step > width && !lines.last().expect("a line").is_empty() {
            let line = lines.last_mut().expect("a line");
            match line.iter().rposition(|g| g.ch == ' ') {
                Some(space) if glyph.ch != ' ' => {
                    let rest = line.split_off(space + 1);
                    line.pop();
                    used = rest.iter().map(|g| g.advance * size).sum();
                    lines.push(rest);
                }
                _ => {
                    lines.push(Vec::new());
                    used = 0.0;
                }
            }
        }
        lines.last_mut().expect("a line").push(glyph);
        used += step;
    }
    lines
}

fn parse(doc: &MuPdf, source: String) -> PdfObject {
    doc.new_object_from_str(&source).expect("object")
}

fn real(value: f32) -> PdfObject {
    PdfObject::new_real(value).expect("real")
}

fn array(doc: &MuPdf, values: &[f32]) -> PdfObject {
    let mut array = doc.new_array().expect("array");
    for &value in values {
        array.array_push(real(value)).expect("push");
    }
    array
}

/// The six capital letters a subset font's name starts with (`ABCDEF+Name`), from its bytes.
fn tag(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    (0..6)
        .map(|i| char::from(b'A' + ((hash >> (i * 5)) % 26) as u8))
        .collect()
}

/// A subset of `data` with the glyphs of `used`, as the PDF objects of a Type0 font over a
/// CIDFontType2, and the numbers the subset gave to those glyphs (they are what the text shows).
fn embed(doc: &mut MuPdf, data: &[u8], used: &[Glyph]) -> (PdfObject, BTreeMap<u16, u16>) {
    let font = FontRef::from_index(data, 0).expect("font");
    let mut remapper = GlyphRemapper::new();
    remapper.remap(0);
    for glyph in used {
        remapper.remap(glyph.id);
    }
    let bytes = subset(data, 0, &remapper).expect("subset");
    let renumbered: BTreeMap<u16, u16> = used
        .iter()
        .map(|glyph| (glyph.id, remapper.get(glyph.id).expect("kept")))
        .collect();
    let per_em = f32::from(font.head().expect("head").units_per_em());
    let scale = 1000.0 / per_em;
    let size = skrifa::instance::Size::unscaled();
    let metrics = font.metrics(size, skrifa::instance::LocationRef::default());
    let bounds = metrics.bounds.expect("bounds");
    let base = format!("{}+TextBoxFont", tag(&bytes));

    let length = parse(doc, format!("<< /Length1 {} >>", bytes.len()));
    let file = doc
        .add_stream(
            &Buffer::from_bytes(&bytes).expect("buffer"),
            Some(&length),
            false,
        )
        .expect("font file");
    let descriptor = parse(
        doc,
        format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags 4 /FontBBox [{} {} {} {}] /ItalicAngle 0 \
         /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {} 0 R >>",
            (bounds.x_min * scale) as i32,
            (bounds.y_min * scale) as i32,
            (bounds.x_max * scale) as i32,
            (bounds.y_max * scale) as i32,
            (metrics.ascent * scale) as i32,
            (metrics.descent * scale) as i32,
            (metrics.cap_height.unwrap_or(metrics.ascent) * scale) as i32,
            file.as_indirect().expect("file"),
        ),
    );
    let descriptor = doc.add_object(&descriptor).expect("descriptor");
    let widths: BTreeMap<u16, i32> = used
        .iter()
        .map(|glyph| {
            (
                renumbered[&glyph.id],
                (glyph.advance * 1000.0).round() as i32,
            )
        })
        .collect();
    let widths: String = widths
        .iter()
        .map(|(id, width)| format!("{id} [{width}] "))
        .collect();
    let descendant = parse(
        doc,
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base} \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
         /FontDescriptor {} 0 R /CIDToGIDMap /Identity /DW 1000 /W [{widths}] >>",
            descriptor.as_indirect().expect("descriptor"),
        ),
    );
    let descendant = doc.add_object(&descendant).expect("descendant");
    let unicode = to_unicode(doc, used, &renumbered);
    let font = parse(
        doc,
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H \
         /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>",
            descendant.as_indirect().expect("descendant"),
            unicode.as_indirect().expect("unicode"),
        ),
    );
    (doc.add_object(&font).expect("font"), renumbered)
}

/// What the glyphs mean, so that other readers can search and copy the text.
fn to_unicode(doc: &mut MuPdf, used: &[Glyph], renumbered: &BTreeMap<u16, u16>) -> PdfObject {
    let mut pairs = BTreeMap::new();
    for glyph in used {
        pairs.insert(renumbered[&glyph.id], glyph.ch);
    }
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def \
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def \
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    cmap.push_str(&format!("{} beginbfchar\n", pairs.len()));
    for (id, ch) in &pairs {
        let mut units = [0u16; 2];
        let text: String = ch
            .encode_utf16(&mut units)
            .iter()
            .map(|unit| format!("{unit:04X}"))
            .collect();
        cmap.push_str(&format!("<{id:04X}> <{text}>\n"));
    }
    cmap.push_str("endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end\n");
    doc.add_stream(
        &Buffer::from_bytes(cmap.as_bytes()).expect("buffer"),
        None,
        false,
    )
    .expect("to unicode")
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

/// The drawing operators of `lines` in a box of `width` x `height` points.
fn drawing(
    lines: &[Vec<Glyph>],
    renumbered: &BTreeMap<u16, u16>,
    size: f32,
    align: Align,
    (width, height): (f32, f32),
) -> String {
    let mut out = format!("BT\n/F0 {size} Tf\n0.1 0.2 0.6 rg\n");
    for (index, line) in lines.iter().enumerate() {
        let used: f32 = line.iter().map(|glyph| glyph.advance * size).sum();
        let x = match align {
            Align::Left => 2.0,
            Align::Center => (width - used) / 2.0,
            Align::Right => width - 2.0 - used,
        };
        let y = height - 2.0 - size - index as f32 * size * 1.2;
        let hex: String = line
            .iter()
            .map(|glyph| format!("{:04X}", renumbered[&glyph.id]))
            .collect();
        out.push_str(&format!("1 0 0 1 {x:.2} {y:.2} Tm <{hex}> Tj\n"));
    }
    out.push_str("ET\n");
    out
}

fn utf16_hex(text: &str) -> String {
    let units: String = text
        .encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect();
    format!("<FEFF{units}>")
}

/// The font of `text` set in the box, and the operators that draw it (in the box's own space).
fn build(doc: &mut MuPdf, data: &[u8], text: &str, size: f32, align: Align) -> (PdfObject, String) {
    let font = FontRef::from_index(data, 0).expect("font");
    let [x0, y0, x1, y1] = BOX;
    let (width, height) = (x1 - x0, y1 - y0);
    let lines = wrap(&shape(&font, text), size, width - 4.0);
    let used: Vec<Glyph> = lines.iter().flatten().copied().collect();
    let (font_object, renumbered) = embed(doc, data, &used);
    let content = drawing(&lines, &renumbered, size, align, (width, height));
    (font_object, content)
}

/// Puts a text box over `BOX` on the first page, by hand: the annotation's dictionary is written
/// here, so that MuPDF never makes an appearance of its own for it. Returns its object number.
fn add_text_box(doc: &mut MuPdf, data: &[u8], text: &str, size: f32, align: Align) -> i32 {
    let [x0, y0, x1, y1] = BOX;
    let (width, height) = (x1 - x0, y1 - y0);
    let (font_object, content) = build(doc, data, text, size, align);
    let form = doc
        .new_object_from_str(&format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 {width} {height}] \
             /Resources << /Font << /F0 {} 0 R >> >> >>",
            font_object.as_indirect().expect("font"),
        ))
        .expect("form");
    let buffer = Buffer::from_bytes(content.as_bytes()).expect("buffer");
    let stream = doc
        .add_stream(&buffer, Some(&form), false)
        .expect("appearance");
    let quadding = match align {
        Align::Left => 0,
        Align::Center => 1,
        Align::Right => 2,
    };
    let annotation = doc
        .new_object_from_str(&format!(
            "<< /Type /Annot /Subtype /FreeText /Rect [{x0} {y0} {x1} {y1}] /Contents {} \
             /DA (/Helv 12 Tf 0 g) /Q {quadding} /F 4 /IT /FreeTextTypeWriter /BS << /W 0 >> \
             /AP << /N {} 0 R >> >>",
            utf16_hex(text),
            stream.as_indirect().expect("stream"),
        ))
        .expect("annotation");
    let annotation = doc.add_object(&annotation).expect("annotation");
    let page = doc.load_pdf_page(0).expect("page");
    let mut page_object = page.object();
    match page_object.get_dict("Annots").expect("annots") {
        Some(mut annots) => annots
            .array_push(annotation.try_clone().expect("clone"))
            .expect("push"),
        None => {
            let mut created = doc.new_array().expect("array");
            created
                .array_push(annotation.try_clone().expect("clone"))
                .expect("push");
            page_object.dict_put("Annots", created).expect("put");
        }
    }
    annotation.as_indirect().expect("number")
}

fn save(doc: &MuPdf) -> Vec<u8> {
    let mut out = Vec::new();
    doc.write_to(&mut out).expect("write");
    out
}

fn blank() -> MuPdf {
    MuPdf::from_bytes(&common::one_page("")).expect("page")
}

/// Dark pixels of `page` inside `area` (left, top, right, bottom, in pixels at 72 dpi).
fn ink(page: &RenderedPage, area: [usize; 4]) -> usize {
    let [x0, y0, x1, y1] = area;
    (y0..y1.min(page.height as usize))
        .flat_map(|y| {
            (x0..x1.min(page.width as usize)).map(move |x| (y * page.width as usize + x) * 4)
        })
        .filter(|&at| {
            page.rgba[at..at + 3]
                .iter()
                .map(|&c| u32::from(c))
                .sum::<u32>()
                < 3 * 160
        })
        .count()
}

/// Where the box is on the rendered page (72 dpi), and the whole page.
const IN_BOX: [usize; 4] = [72, 72, 372, 132];
const WHOLE_PAGE: [usize; 4] = [0, 0, 612, 792];

/// Opens `bytes`, takes the first annotation of the first page and tells what its appearance has
/// as fonts: their resource names, and the `BaseFont` of the first.
fn appearance_fonts(bytes: &[u8]) -> (Vec<String>, Vec<u8>) {
    let doc = MuPdf::from_bytes(bytes).expect("open");
    let page = doc.load_pdf_page(0).expect("page");
    let annotation = page.annotations().next().expect("an annotation");
    let fonts = annotation
        .object()
        .get_dict("AP")
        .expect("ap")
        .and_then(|ap| ap.get_dict("N").expect("n"))
        .and_then(|n| n.get_dict("Resources").expect("resources"))
        .and_then(|resources| resources.get_dict("Font").expect("font"))
        .expect("fonts");
    let names: Vec<String> = (0..fonts.dict_len().expect("len") as i32)
        .map(|at| {
            let key = fonts.get_dict_key(at).expect("key").expect("key");
            String::from_utf8_lossy(&key.as_name().expect("name")).into_owned()
        })
        .collect();
    let first = fonts.get_dict_val(0).expect("val").expect("val");
    let base = first
        .get_dict("BaseFont")
        .expect("base")
        .map(|base| base.as_name().expect("name"))
        .unwrap_or_default();
    (names, base)
}

#[test]
fn a_text_box_is_drawn_from_a_subset_and_survives_a_save() {
    let mut doc = blank();
    add_text_box(&mut doc, droid(), MIXED, 24.0, Align::Left);
    let bytes = save(&doc);
    // The font file is 3.5 MB; what the file holds of it is the glyphs the box uses.
    assert!(bytes.len() < 20_000, "{} bytes", bytes.len());

    let page = PdfDocument::from_bytes(&bytes)
        .expect("open")
        .render(0, 1.0, 0)
        .expect("render");
    let inside = ink(&page, IN_BOX);
    assert!(inside > 100, "the box is drawn: {inside}");
    assert_eq!(ink(&page, WHOLE_PAGE), inside, "and nothing else is");

    // What another reader finds: a FreeText annotation that says its text, in a subset font.
    let reopened = MuPdf::from_bytes(&bytes).expect("open");
    let page = reopened.load_pdf_page(0).expect("page");
    let annotation = page.annotations().next().expect("an annotation");
    let subtype = annotation.object().get_dict("Subtype").expect("subtype");
    assert_eq!(
        subtype.expect("subtype").as_name().expect("name"),
        b"FreeText"
    );
    assert_eq!(annotation.contents().expect("contents"), Some(MIXED));
    let (names, base) = appearance_fonts(&bytes);
    assert_eq!(names, ["F0"]);
    assert_eq!(base[6], b'+', "{}", String::from_utf8_lossy(&base));
    assert!(base[..6].iter().all(u8::is_ascii_uppercase));
}

#[test]
fn moving_a_text_box_changes_its_rect_and_nothing_else() {
    let mut doc = blank();
    let number = add_text_box(&mut doc, droid(), MIXED, 24.0, Align::Left);
    let mut annotation = doc
        .new_indirect(number, 0)
        .expect("reference")
        .resolve()
        .expect("resolve")
        .expect("annotation");
    let moved = array(
        &doc,
        &[172.0, PAGE_HEIGHT - 332.0, 472.0, PAGE_HEIGHT - 272.0],
    );
    annotation.dict_put("Rect", moved).expect("rect");
    let bytes = save(&doc);
    let page = PdfDocument::from_bytes(&bytes)
        .expect("open")
        .render(0, 1.0, 0)
        .expect("render");
    assert!(ink(&page, [172, 272, 472, 332]) > 100);
    assert_eq!(ink(&page, IN_BOX), 0);
    assert_eq!(appearance_fonts(&bytes).0, ["F0"]);
}

/// Blue pixels of `page` inside `area`: the colour the test's text is set in (black is MuPDF's).
fn blue_ink(page: &RenderedPage, area: [usize; 4]) -> usize {
    let [x0, y0, x1, y1] = area;
    (y0..y1.min(page.height as usize))
        .flat_map(|y| {
            (x0..x1.min(page.width as usize)).map(move |x| (y * page.width as usize + x) * 4)
        })
        .filter(|&at| {
            let [r, g, b] = [page.rgba[at], page.rgba[at + 1], page.rgba[at + 2]].map(i32::from);
            b > r + 40 && b > g + 20
        })
        .count()
}

#[test]
fn mupdf_draws_the_appearance_again_when_it_is_asked_to_change_the_annotation() {
    let mut doc = blank();
    add_text_box(&mut doc, droid(), MIXED, 24.0, Align::Left);
    let first = save(&doc);
    let render = |bytes: &[u8]| {
        PdfDocument::from_bytes(bytes)
            .expect("open")
            .render(0, 1.0, 0)
            .expect("render")
    };
    let before = blue_ink(&render(&first), WHOLE_PAGE);
    assert!(before > 100, "{before}");

    // As the worker does for an edit: open the document, change the annotation, update it.
    for change in ["contents", "rect"] {
        let doc = MuPdf::from_bytes(&first).expect("open");
        {
            let page = doc.load_pdf_page(0).expect("page");
            let mut annotation = page.annotations().next().expect("an annotation");
            match change {
                "contents" => annotation.set_contents("Hello 你好").expect("contents"),
                _ => {
                    let mut rect = annotation.rect().expect("rect");
                    rect.y0 += 1.0;
                    rect.y1 += 1.0;
                    annotation.set_rect(rect).expect("rect");
                }
            }
            annotation.update().expect("update");
        }
        let second = save(&doc);
        // MuPDF's own layout: the text is black, in the annotation's default font (`/DA`),
        // not in the font and colour the box was made with.
        let after = blue_ink(&render(&second), WHOLE_PAGE);
        assert!(after < before / 10, "{change}: {before} -> {after}");
    }
}

#[test]
fn the_text_of_a_text_box_is_not_in_the_text_of_the_page() {
    let mut doc = blank();
    add_text_box(&mut doc, droid(), MIXED, 24.0, Align::Left);
    let app = PdfDocument::from_bytes(&save(&doc)).expect("open");
    for query in ["Hello", "你好"] {
        let found = app.search_page(0, query, true, 5).expect("search");
        assert!(found.hits.is_empty(), "{query}");
    }
}

#[test]
fn the_same_text_as_page_content_is_found_through_its_to_unicode_map() {
    let mut doc = blank();
    let (font, content) = build(&mut doc, droid(), MIXED, 24.0, Align::Left);
    let [x0, y0, ..] = BOX;
    let nl = char::from(10u8);
    let moved = format!("q 1 0 0 1 {x0} {y0} cm{nl}{content}Q{nl}");
    let buffer = Buffer::from_bytes(moved.as_bytes()).expect("buffer");
    let stream = doc.add_stream(&buffer, None, false).expect("content");
    let resources = parse(
        &doc,
        format!(
            "<< /Font << /F0 {} 0 R >> >>",
            font.as_indirect().expect("font")
        ),
    );
    let page = doc.load_pdf_page(0).expect("page");
    let mut page_object = page.object();
    page_object
        .dict_put("Resources", resources)
        .expect("resources");
    page_object.dict_put("Contents", stream).expect("contents");
    let app = PdfDocument::from_bytes(&save(&doc)).expect("open");
    for query in ["Hello", "你好", "世界"] {
        let found = app.search_page(0, query, true, 5).expect("search");
        assert_eq!(found.hits.len(), 1, "{query}");
    }
}

#[test]
fn chinese_text_without_spaces_wraps_inside_the_box() {
    let font = FontRef::from_index(droid(), 0).expect("font");
    let text = format!(
        "{}{}{}",
        "這是一段沒有空白的中文，要在方框裡自己斷行。",
        char::from(10u8),
        "Second paragraph in English, which breaks at spaces."
    );
    let text = text.as_str();
    let (size, width) = (16.0, 200.0);
    let lines = wrap(&shape(&font, text), size, width);
    assert!(lines.len() >= 5, "{}", lines.len());
    for line in &lines {
        let used: f32 = line.iter().map(|glyph| glyph.advance * size).sum();
        assert!(used <= width, "{used}");
        assert!(line.first().is_none_or(|glyph| glyph.ch != ' '));
    }
    // The text is all there, in order, apart from the break and the spaces taken out.
    let again: String = lines.iter().flatten().map(|glyph| glyph.ch).collect();
    assert_eq!(
        again.replace(' ', ""),
        text.replace([char::from(10u8), ' '], "")
    );
}

/// MuPDF's own TrueType subsetter (what `mutool clean -s` uses), on a context of its own: an error
/// ends the process there, which is fine for a measurement and is the reason not to use it.
fn mupdf_subset(font: &[u8], glyphs: &mut Vec<i32>) -> Vec<u8> {
    use mupdf_sys::{
        FZ_STORE_DEFAULT, fz_buffer_storage, fz_drop_buffer, fz_drop_context,
        fz_new_buffer_from_shared_data, fz_new_context_imp, fz_subset_ttf_for_gids,
    };
    glyphs.sort_unstable();
    glyphs.dedup();
    // SAFETY: the context, buffers and the font's bytes outlive every call, on this thread only.
    unsafe {
        let ctx = fz_new_context_imp(
            std::ptr::null(),
            std::ptr::null(),
            FZ_STORE_DEFAULT as usize,
            c"1.27.2".as_ptr(),
        );
        assert!(!ctx.is_null());
        let buffer = fz_new_buffer_from_shared_data(ctx, font.as_ptr(), font.len());
        let count = i32::try_from(glyphs.len()).expect("count");
        let subset = fz_subset_ttf_for_gids(ctx, buffer, glyphs.as_mut_ptr(), count, 0, 1);
        let mut data: *mut u8 = std::ptr::null_mut();
        let length = fz_buffer_storage(ctx, subset, &mut data);
        let out = std::slice::from_raw_parts(data, length).to_vec();
        fz_drop_buffer(ctx, subset);
        fz_drop_buffer(ctx, buffer);
        fz_drop_context(ctx);
        out
    }
}

#[test]
fn a_subset_is_a_few_kilobytes_from_the_subsetter_and_hundreds_from_mupdfs() {
    let data = droid();
    let font = FontRef::from_index(data, 0).expect("font");
    let glyphs = shape(&font, MIXED);
    let mut remapper = GlyphRemapper::new();
    remapper.remap(0);
    for glyph in &glyphs {
        remapper.remap(glyph.id);
    }
    let small = subset(data, 0, &remapper).expect("subset");
    let mut ids: Vec<i32> = std::iter::once(0)
        .chain(glyphs.iter().map(|glyph| i32::from(glyph.id)))
        .collect();
    let big = mupdf_subset(data, &mut ids);
    // The font is 3.5 MB. MuPDF keeps every glyph's slot (and the tables that list them): a
    // text box with ten glyphs would put 400 KB in the file, and in the crash-recovery journal.
    assert!(data.len() > 3_000_000);
    assert!(small.len() < 5_000, "{}", small.len());
    assert!(big.len() > 100_000, "{}", big.len());
    assert!(big.len() > 100 * small.len());

    // A sentence of fifty characters is still a few kilobytes: the crash-recovery journal holds
    // 4 MiB, so it holds a few hundred such boxes, where MuPDF's would fill it with ten.
    let sentence =
        "The quick brown fox jumps over the lazy dog 你好，世界！這是一段比較長的中文文字。";
    let mut remapper = GlyphRemapper::new();
    remapper.remap(0);
    for glyph in shape(&font, sentence) {
        remapper.remap(glyph.id);
    }
    let longer = subset(data, 0, &remapper).expect("subset");
    assert!(longer.len() < 8_000, "{}", longer.len());
}

/// What a font's licence says about putting it in a PDF (OS/2 `fsType`, OpenType specification).
#[derive(Debug, PartialEq)]
enum Embedding {
    /// Installable or editable embedding.
    Allowed,
    /// Preview and print embedding: the PDF may only be viewed and printed, not edited.
    ViewOnly,
    /// Restricted licence, bitmaps only, or no subsetting (which the app always does).
    Refused,
}

fn embedding(fs_type: u16) -> Embedding {
    const RESTRICTED: u16 = 0x0002;
    const PREVIEW_AND_PRINT: u16 = 0x0004;
    const NO_SUBSETTING: u16 = 0x0100;
    const BITMAP_ONLY: u16 = 0x0200;
    if fs_type & (RESTRICTED | NO_SUBSETTING | BITMAP_ONLY) != 0 {
        Embedding::Refused
    } else if fs_type & PREVIEW_AND_PRINT != 0 {
        Embedding::ViewOnly
    } else {
        Embedding::Allowed
    }
}

#[test]
fn a_fonts_licence_for_embedding_can_be_read_from_the_font() {
    let font = FontRef::from_index(droid(), 0).expect("font");
    let fs_type = font.os2().expect("an OS/2 table").fs_type();
    assert_eq!(embedding(fs_type), Embedding::Allowed, "{fs_type:#06x}");
    for (fs_type, expected) in [
        (0x0000, Embedding::Allowed),
        (0x0008, Embedding::Allowed),
        (0x0004, Embedding::ViewOnly),
        (0x0002, Embedding::Refused),
        (0x0100, Embedding::Refused),
        (0x0108, Embedding::Refused),
        (0x0200, Embedding::Refused),
    ] {
        assert_eq!(embedding(fs_type), expected, "{fs_type:#06x}");
    }
}

/// The fonts Windows lists as installed, as the main process would: the registry says each
/// font's name, and its file, without a font file being opened.
#[cfg(windows)]
#[test]
fn the_installed_fonts_are_listed_by_the_registry_without_reading_a_font_file() {
    let slash = char::from(92u8);
    let key = [
        "HKLM",
        "SOFTWARE",
        "Microsoft",
        "Windows NT",
        "CurrentVersion",
        "Fonts",
    ]
    .join(&slash.to_string());
    let output = std::process::Command::new("reg")
        .args(["query", &key])
        .output()
        .expect("reg");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    let fonts: Vec<(&str, &str)> = text
        .lines()
        .filter_map(|line| line.trim().split_once("    REG_SZ    "))
        .collect();
    assert!(fonts.len() > 20, "{} fonts", fonts.len());
    // Every font is a name, with its kind in parentheses, and a file.
    assert!(fonts.iter().any(|(name, file)| {
        name.ends_with(" (TrueType)") && file.to_ascii_lowercase().ends_with(".ttf")
    }));
    // The bold and italic faces of a family are entries of their own, with their own files.
    assert!(fonts.iter().any(|(name, _)| name.contains(" Bold")));
    assert!(fonts.iter().any(|(name, _)| name.contains(" Italic")));
}

#[test]
fn the_app_lists_a_text_box_keeps_it_when_it_saves_and_removes_its_font_with_it() {
    use ipc_contract::types::AnnotationKind;

    let mut doc = blank();
    add_text_box(&mut doc, droid(), MIXED, 24.0, Align::Left);
    let mut app = PdfDocument::from_bytes(&save(&doc)).expect("open");
    // The selection, move and removal of B2-07 and B2-08 already see it, as an annotation of a
    // kind they only know how to remove.
    let listed = app.page_annotations(0).expect("annotations");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind, AnnotationKind::Other);
    let rect = listed[0].rect;
    assert_eq!(
        [rect.x0, rect.y0, rect.x1, rect.y1],
        [72.0, 72.0, 372.0, 132.0]
    );
    assert!(app.set_annotation_rect(0, listed[0].id, rect).is_err());

    // The app's own save keeps the appearance and its subset.
    let mut saved = Vec::new();
    app.save(&mut saved).expect("save");
    assert!(saved.len() < 20_000, "{}", saved.len());
    assert_eq!(appearance_fonts(&saved).0, ["F0"]);

    // Removing the text box takes its font with it: nothing points to it, so a rewrite drops it.
    app.delete_annotation(0, listed[0].id).expect("delete");
    let mut gone = Vec::new();
    app.save(&mut gone).expect("save");
    let blank_size = save(&blank()).len();
    assert!(
        gone.len() < blank_size + 300,
        "{} against {blank_size}",
        gone.len()
    );
}

#[test]
fn a_free_text_annotation_made_by_mupdf_has_a_callout_and_no_appearance_in_the_file() {
    let doc = blank();
    {
        let mut page = doc.load_pdf_page(0).expect("page");
        let rect = mupdf::Rect {
            x0: 72.0,
            y0: 72.0,
            x1: 372.0,
            y1: 132.0,
        };
        page.add_free_text_annotation(rect, MIXED)
            .expect("annotation");
    }
    let bytes = save(&doc);
    let reopened = MuPdf::from_bytes(&bytes).expect("open");
    let page = reopened.load_pdf_page(0).expect("page");
    let annotation = page.annotations().next().expect("an annotation");
    // A callout line nobody asked for, and no appearance: MuPDF draws the text when it shows the
    // page, with `/DA`'s font (`Helv`) and a CJK substitute, never in the font the user chose.
    let dict = annotation.object();
    assert!(dict.get_dict("CL").expect("cl").is_some());
    assert!(dict.get_dict("AP").expect("ap").is_none());
}

#[test]
fn mupdfs_add_font_embeds_the_whole_font() {
    let mut doc = blank();
    let font = Font::from_bytes("DroidSansFallback", droid()).expect("font");
    doc.add_font(&font).expect("embed");
    let bytes = save(&doc);
    assert!(bytes.len() > 3_000_000, "{}", bytes.len());
}

/// The first and the last column (of the page at 72 dpi) that has dark pixels in the box.
fn ink_span(page: &RenderedPage) -> (usize, usize) {
    let [x0, y0, x1, y1] = IN_BOX;
    let columns: Vec<usize> = (x0..x1)
        .filter(|&x| ink(page, [x, y0, x + 1, y1]) > 0)
        .collect();
    (
        *columns.first().expect("ink"),
        *columns.last().expect("ink"),
    )
}

#[test]
fn text_is_set_at_the_left_in_the_middle_or_at_the_right_of_the_box() {
    let [x0, _, x1, _] = IN_BOX.map(|edge| edge as f32);
    let mut spans = Vec::new();
    for align in [Align::Left, Align::Center, Align::Right] {
        let mut doc = blank();
        add_text_box(&mut doc, droid(), "Hello", 24.0, align);
        let page = PdfDocument::from_bytes(&save(&doc))
            .expect("open")
            .render(0, 1.0, 0)
            .expect("render");
        spans.push(ink_span(&page));
    }
    let [left, middle, right] = [spans[0], spans[1], spans[2]];
    assert!(left.0 as f32 <= x0 + 6.0, "{left:?}");
    assert!(right.1 as f32 >= x1 - 6.0, "{right:?}");
    let centre = (middle.0 + middle.1) as f32 / 2.0;
    assert!((centre - (x0 + x1) / 2.0).abs() <= 6.0, "{middle:?}");
    // The same text, so the same width in all three.
    let widths = [left, middle, right].map(|(first, last)| last as i32 - first as i32);
    assert!(
        widths.iter().all(|width| (width - widths[0]).abs() <= 2),
        "{widths:?}"
    );
}
