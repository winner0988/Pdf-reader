//! POC for Issue #593 (B6-08): editing existing page text directly in its original font, size,
//! and colour — and editing text on scanned pages (Option 1: background fill + new visible text).
//!
//! **Do not merge into `main`.** This lives on a POC branch / draft PR to measure feasibility,
//! collect numbers across `tests/corpus` and synthetic samples, document MuPDF/binding gaps, and
//! produce sample PDFs + PNGs for the owner to inspect in Acrobat Reader and Edge.
//!
//! Verification uses the worker's own `pdf_worker::engine::PdfDocument` (the same code path the
//! app uses for search, text extraction, rendering, permissions, signature detection, and saving)
//! and `qpdf --check` (`common::check_qpdf_bytes`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use ipc_contract::types::Rgb;
use jpeg_encoder::{ChromaSubsamplingMethod, ColorType, Encoder};
use mupdf::pdf::{
    PdfDocument as MuPdf, PdfObject, PdfPage, PdfRedactImageMethod, PdfRedactLineArtMethod,
    PdfRedactOptions, PdfRedactTextMethod, PdfWriteOptions,
};
use mupdf::text_page::{TextBlockType, TextCharFlags};
use mupdf::{Buffer, Point, Quad, Rect, TextPageFlags, WriteMode};
use pdf_worker::engine::{PdfDocument, RenderedPage};
use skrifa::raw::TableProvider;
use skrifa::raw::types::Tag;
use skrifa::string::StringId;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use subsetter::{GlyphRemapper, subset};

#[allow(dead_code)]
mod common;

// --------------------------------------------------------------------------- paths & output

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn corpus_bytes(rel: &str) -> Vec<u8> {
    let path = corpus_root().join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn poc_out_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("poc-593");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn save_poc_artifact(name: &str, bytes: &[u8]) {
    let path = poc_out_dir().join(name);
    let _ = std::fs::write(&path, bytes);
}

fn save_poc_pair(stem: &str, before_pdf: &[u8], after_pdf: &[u8], page: u32) {
    save_poc_artifact(&format!("{stem}-before.pdf"), before_pdf);
    save_poc_artifact(&format!("{stem}-after.pdf"), after_pdf);
    if let Ok(doc) = PdfDocument::from_bytes(before_pdf)
        && let Ok(png) = doc.render_png(page, 2.0)
    {
        save_poc_artifact(&format!("{stem}-before.png"), &png);
    }
    if let Ok(doc) = PdfDocument::from_bytes(after_pdf)
        && let Ok(png) = doc.render_png(page, 2.0)
    {
        save_poc_artifact(&format!("{stem}-after.png"), &png);
    }
}

fn bundled_droid() -> &'static [u8] {
    mupdf_fonts_droid::find_by_name("Droid Sans Fallback", false, false)
        .expect("bundled Droid Sans Fallback")
        .data
}

fn save_mupdf(doc: &MuPdf, compress: bool) -> Vec<u8> {
    let mut opts = PdfWriteOptions::default();
    opts.set_garbage(true);
    opts.set_compress(compress);
    let mut out = Vec::new();
    doc.write_to_with_options(&mut out, opts).expect("write");
    out
}

fn parse_obj(doc: &MuPdf, source: &str) -> PdfObject {
    doc.new_object_from_str(source).expect("parse pdf object")
}

// --------------------------------------------------------------------------- A: font inspection

#[derive(Debug, Clone, PartialEq, Eq)]
enum EncodingKind {
    WinAnsi,
    MacRoman,
    Standard,
    MacExpert,
    IdentityH,
    IdentityV,
    NamedCMap(String),
    EmbeddedCMap,
    CustomDifferences {
        base: Option<String>,
        diff_count: usize,
        has_non_standard_names: bool,
    },
    Implicit,
}

impl EncodingKind {
    fn label(&self) -> String {
        match self {
            Self::WinAnsi => "WinAnsiEncoding".to_owned(),
            Self::MacRoman => "MacRomanEncoding".to_owned(),
            Self::Standard => "StandardEncoding".to_owned(),
            Self::MacExpert => "MacExpertEncoding".to_owned(),
            Self::IdentityH => "Identity-H".to_owned(),
            Self::IdentityV => "Identity-V".to_owned(),
            Self::NamedCMap(name) => name.clone(),
            Self::EmbeddedCMap => "EmbeddedCMap".to_owned(),
            Self::CustomDifferences {
                base, diff_count, ..
            } => format!(
                "Differences({}, {diff_count})",
                base.as_deref().unwrap_or("none")
            ),
            Self::Implicit => "Implicit".to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
struct PdfFontResource {
    resource_key: String,
    raw_name: String,
    clean_name: String,
    is_subset: bool,
    subtype: String,
    descendant_subtype: Option<String>,
    embedded: bool,
    encoding: EncodingKind,
    has_to_unicode: bool,
    /// Reverse map parsed from `/ToUnicode`: Unicode char -> character code bytes.
    unicode_to_code: BTreeMap<char, Vec<u8>>,
    /// Embedded `FontFile2` bytes (TrueType), if present.
    font_file2: Option<Vec<u8>>,
    fd_flags: Option<i32>,
    xref: Option<i32>,
}

fn strip_subset_prefix(raw: &str) -> (&str, bool) {
    let trimmed = raw.trim_start_matches('/');
    if trimmed.len() > 7
        && trimmed.as_bytes()[6] == b'+'
        && trimmed.as_bytes()[..6]
            .iter()
            .all(|b| b.is_ascii_uppercase())
    {
        (&trimmed[7..], true)
    } else {
        (trimmed, false)
    }
}

fn is_standard_glyph_name(name: &str) -> bool {
    if name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric() {
        return true;
    }
    matches!(
        name,
        "space"
            | "exclam"
            | "quotedbl"
            | "numbersign"
            | "dollar"
            | "percent"
            | "ampersand"
            | "quotesingle"
            | "parenleft"
            | "parenright"
            | "asterisk"
            | "plus"
            | "comma"
            | "hyphen"
            | "period"
            | "slash"
            | "zero"
            | "one"
            | "two"
            | "three"
            | "four"
            | "five"
            | "six"
            | "seven"
            | "eight"
            | "nine"
            | "colon"
            | "semicolon"
            | "less"
            | "equal"
            | "greater"
            | "question"
            | "at"
            | "bracketleft"
            | "backslash"
            | "bracketright"
            | "asciicircum"
            | "underscore"
            | "grave"
            | "braceleft"
            | "bar"
            | "braceright"
            | "asciitilde"
            | "fi"
            | "fl"
            | "endash"
            | "emdash"
            | "bullet"
            | "quoteleft"
            | "quoteright"
            | "quotedblleft"
            | "quotedblright"
            | "Euro"
            | "minus"
            | "copyright"
            | "registered"
            | "trademark"
            | "nbspace"
    ) || name.starts_with("uni")
}

fn parse_encoding(font_obj: &PdfObject) -> EncodingKind {
    let Ok(Some(enc)) = font_obj.get_dict("Encoding") else {
        return EncodingKind::Implicit;
    };
    if enc.is_name().unwrap_or(false)
        && let Ok(bytes) = enc.as_name()
    {
        let name = String::from_utf8_lossy(&bytes).into_owned();
        return match name.as_str() {
            "WinAnsiEncoding" => EncodingKind::WinAnsi,
            "MacRomanEncoding" => EncodingKind::MacRoman,
            "StandardEncoding" => EncodingKind::Standard,
            "MacExpertEncoding" => EncodingKind::MacExpert,
            "Identity-H" => EncodingKind::IdentityH,
            "Identity-V" => EncodingKind::IdentityV,
            _ => EncodingKind::NamedCMap(name),
        };
    }
    if enc.is_stream().unwrap_or(false) {
        return EncodingKind::EmbeddedCMap;
    }
    if enc.is_dict().unwrap_or(false) {
        let base = enc
            .get_dict("BaseEncoding")
            .ok()
            .flatten()
            .and_then(|b| b.as_name().ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let mut diff_count = 0usize;
        let mut has_non_standard_names = false;
        if let Ok(Some(diffs)) = enc.get_dict("Differences")
            && let Ok(iter) = diffs.array_iter()
        {
            for item in iter.flatten() {
                if item.is_name().unwrap_or(false)
                    && let Ok(n) = item.as_name()
                {
                    diff_count += 1;
                    let s = String::from_utf8_lossy(&n);
                    if !is_standard_glyph_name(&s) {
                        has_non_standard_names = true;
                    }
                }
            }
        }
        return EncodingKind::CustomDifferences {
            base,
            diff_count,
            has_non_standard_names,
        };
    }
    EncodingKind::Implicit
}

fn parse_hex_bytes(token: &str) -> Option<Vec<u8>> {
    let hex = token.trim().strip_prefix('<')?.strip_suffix('>')?;
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        out.push(u8::from_str_radix(&hex[i..i + 2], 16).ok()?);
    }
    Some(out)
}

fn decode_utf16be_char(bytes: &[u8]) -> Option<char> {
    if bytes.len() == 2 {
        let unit = u16::from_be_bytes([bytes[0], bytes[1]]);
        char::from_u32(u32::from(unit))
    } else if bytes.len() == 4 {
        let hi = u16::from_be_bytes([bytes[0], bytes[1]]);
        let lo = u16::from_be_bytes([bytes[2], bytes[3]]);
        String::from_utf16(&[hi, lo]).ok()?.chars().next()
    } else {
        None
    }
}

/// Minimal `/ToUnicode` CMap parser (`beginbfchar` + `beginbfrange`) building a reverse map
/// `Unicode char -> source code bytes` so we can test whether an existing font subset can
/// encode replacement text.
fn parse_to_unicode_reverse(cmap_bytes: &[u8]) -> BTreeMap<char, Vec<u8>> {
    let text = String::from_utf8_lossy(cmap_bytes);
    let mut map = BTreeMap::new();
    let mut mode = 0u8; // 1 = bfchar, 2 = bfrange
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.contains("beginbfchar") {
            mode = 1;
            continue;
        }
        if line.contains("endbfchar") || line.contains("endbfrange") {
            mode = 0;
            continue;
        }
        if line.contains("beginbfrange") {
            mode = 2;
            continue;
        }
        let tokens: Vec<&str> = line
            .split_whitespace()
            .filter(|t| t.starts_with('<') && t.ends_with('>'))
            .collect();
        if mode == 1 && tokens.len() >= 2 {
            if let (Some(src), Some(dst)) = (parse_hex_bytes(tokens[0]), parse_hex_bytes(tokens[1]))
                && let Some(ch) = decode_utf16be_char(&dst)
            {
                map.entry(ch).or_insert(src);
            }
        } else if mode == 2
            && tokens.len() >= 3
            && let (Some(start), Some(end), Some(dst)) = (
                parse_hex_bytes(tokens[0]),
                parse_hex_bytes(tokens[1]),
                parse_hex_bytes(tokens[2]),
            )
            && start.len() == end.len()
            && start.len() <= 2
            && dst.len() == 2
        {
            let s = if start.len() == 1 {
                u32::from(start[0])
            } else {
                u32::from(u16::from_be_bytes([start[0], start[1]]))
            };
            let e = if end.len() == 1 {
                u32::from(end[0])
            } else {
                u32::from(u16::from_be_bytes([end[0], end[1]]))
            };
            let d0 = u32::from(u16::from_be_bytes([dst[0], dst[1]]));
            for offset in 0..=(e.saturating_sub(s)).min(512) {
                if let Some(ch) = char::from_u32(d0 + offset) {
                    let code = s + offset;
                    let bytes = if start.len() == 1 {
                        vec![code as u8]
                    } else {
                        (code as u16).to_be_bytes().to_vec()
                    };
                    map.entry(ch).or_insert(bytes);
                }
            }
        }
    }
    map
}

fn inspect_font_dict(key: String, font_obj: &PdfObject) -> PdfFontResource {
    let subtype = font_obj
        .get_dict("Subtype")
        .ok()
        .flatten()
        .and_then(|s| s.as_name().ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_else(|| "Unknown".to_owned());

    let raw_name = font_obj
        .get_dict("BaseFont")
        .ok()
        .flatten()
        .and_then(|bf| bf.as_name().ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .or_else(|| {
            font_obj
                .get_dict("Name")
                .ok()
                .flatten()
                .and_then(|n| n.as_name().ok())
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        })
        .unwrap_or_else(|| {
            if subtype == "Type3" {
                "Type3".to_owned()
            } else {
                "Unnamed".to_owned()
            }
        });

    let (clean, mut is_subset) = strip_subset_prefix(&raw_name);
    let mut clean_name = clean.to_owned();

    let mut descendant_subtype = None;
    let descriptor = if subtype == "Type0" {
        font_obj
            .get_dict("DescendantFonts")
            .ok()
            .flatten()
            .and_then(|arr| arr.get_array(0).ok().flatten())
            .and_then(|desc_font| {
                descendant_subtype = desc_font
                    .get_dict("Subtype")
                    .ok()
                    .flatten()
                    .and_then(|s| s.as_name().ok())
                    .map(|b| String::from_utf8_lossy(&b).into_owned());
                if !is_subset
                    && let Ok(Some(bf)) = desc_font.get_dict("BaseFont")
                    && let Ok(bytes) = bf.as_name()
                {
                    let desc_raw = String::from_utf8_lossy(&bytes);
                    let (desc_clean, desc_sub) = strip_subset_prefix(&desc_raw);
                    if desc_sub {
                        is_subset = true;
                        clean_name = desc_clean.to_owned();
                    }
                }
                desc_font.get_dict("FontDescriptor").ok().flatten()
            })
    } else {
        font_obj.get_dict("FontDescriptor").ok().flatten()
    };

    let mut embedded = subtype == "Type3";
    let mut font_file2 = None;
    let mut fd_flags = None;
    if let Some(desc) = &descriptor {
        fd_flags = desc
            .get_dict("Flags")
            .ok()
            .flatten()
            .and_then(|f| f.as_int().ok());
        if let Ok(Some(ff2)) = desc.get_dict("FontFile2") {
            embedded = true;
            if let Ok(bytes) = ff2.read_stream() {
                font_file2 = Some(bytes);
            }
        } else if desc.get_dict("FontFile").ok().flatten().is_some()
            || desc.get_dict("FontFile3").ok().flatten().is_some()
        {
            embedded = true;
        }
    }

    let encoding = parse_encoding(font_obj);
    let to_unicode_obj = font_obj.get_dict("ToUnicode").ok().flatten();
    let has_to_unicode = to_unicode_obj.is_some();
    let unicode_to_code = to_unicode_obj
        .and_then(|tu| tu.read_stream().ok())
        .map(|bytes| parse_to_unicode_reverse(&bytes))
        .unwrap_or_default();

    PdfFontResource {
        resource_key: key,
        raw_name,
        clean_name,
        is_subset,
        subtype,
        descendant_subtype,
        embedded,
        encoding,
        has_to_unicode,
        unicode_to_code,
        font_file2,
        fd_flags,
        xref: font_obj.as_indirect().ok().filter(|&id| id > 0),
    }
}

fn collect_resources_fonts(
    res_obj: &PdfObject,
    depth: usize,
    seen_res: &mut HashSet<i32>,
    seen_xobj: &mut HashSet<i32>,
    out: &mut Vec<PdfFontResource>,
) {
    if depth > 8 {
        return;
    }
    if let Ok(id) = res_obj.as_indirect()
        && id > 0
        && !seen_res.insert(id)
    {
        return;
    }
    if let Ok(Some(fonts)) = res_obj.get_dict("Font")
        && let Ok(iter) = fonts.dict_iter()
    {
        for item in iter.flatten() {
            let (k, v) = item;
            let key = k
                .as_name()
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            out.push(inspect_font_dict(key, &v));
        }
    }
    if let Ok(Some(xobjs)) = res_obj.get_dict("XObject")
        && let Ok(iter) = xobjs.dict_iter()
    {
        for item in iter.flatten() {
            let (_, xobj) = item;
            let is_form = xobj
                .get_dict("Subtype")
                .ok()
                .flatten()
                .and_then(|s| s.as_name().ok())
                .is_some_and(|n| n == b"Form");
            if !is_form {
                continue;
            }
            if let Ok(id) = xobj.as_indirect()
                && id > 0
                && !seen_xobj.insert(id)
            {
                continue;
            }
            if let Ok(Some(xres)) = xobj.get_dict("Resources") {
                collect_resources_fonts(&xres, depth + 1, seen_res, seen_xobj, out);
            }
        }
    }
}

fn collect_page_fonts(page: &PdfPage) -> Vec<PdfFontResource> {
    let page_obj = page.object();
    let res = page_obj
        .get_dict("Resources")
        .ok()
        .flatten()
        .or_else(|| page_obj.get_dict_inheritable("Resources").ok().flatten());
    let mut out = Vec::new();
    if let Some(res) = res {
        let mut seen_res = HashSet::new();
        let mut seen_xobj = HashSet::new();
        collect_resources_fonts(&res, 0, &mut seen_res, &mut seen_xobj, &mut out);
    }
    out
}

fn match_pdf_font_resource<'a>(
    fz_name: &str,
    resources: &'a [PdfFontResource],
) -> Option<&'a PdfFontResource> {
    let (clean_fz, _) = strip_subset_prefix(fz_name);
    // 1. Exact raw name match
    if let Some(found) = resources.iter().find(|r| r.raw_name == fz_name) {
        return Some(found);
    }
    // 2. Clean name match (case-insensitive)
    if let Some(found) = resources
        .iter()
        .find(|r| r.clean_name.eq_ignore_ascii_case(clean_fz))
    {
        return Some(found);
    }
    // 3. Type3 fonts: MuPDF formats unnamed Type3 fonts as `"Type3 (<xref> 0 R)"` (pdf-type3.c:74)
    if let Some(rest) = fz_name.strip_prefix("Type3 (")
        && let Some(num_str) = rest.split_whitespace().next()
        && let Ok(xref_num) = num_str.parse::<i32>()
        && let Some(found) = resources
            .iter()
            .find(|r| r.subtype == "Type3" && r.xref == Some(xref_num))
    {
        return Some(found);
    }
    if (fz_name.starts_with("Type3") || fz_name.is_empty())
        && let Some(found) = resources.iter().find(|r| r.subtype == "Type3")
    {
        return Some(found);
    }
    // 4. Non-embedded CJK substituted with bundled Droid Sans Fallback -> match non-embedded Type0
    let lower_fz = clean_fz.to_ascii_lowercase();
    if (lower_fz.starts_with("droidsansfallback") || lower_fz.starts_with("droid sans fallback"))
        && let Some(found) = resources
            .iter()
            .find(|r| r.subtype == "Type0" && !r.embedded)
    {
        return Some(found);
    }
    // 5. Single font on page fallback when MuPDF substituted a generic name
    if resources.len() == 1 {
        return resources.first();
    }
    None
}

// --------------------------------------------------------------------------- A: Windows font matcher

#[derive(Debug, Clone)]
struct InstalledFace {
    path: PathBuf,
    face_index: u32,
    postscript_name: String,
    id1_families: Vec<String>,
    families: Vec<String>,
    full_names: Vec<String>,
    subfamily_lower: String,
    weight_class: u16,
    width_class: u16,
    bold: bool,
    italic: bool,
    monospaced: bool,
    serif: bool,
    truetype_embeddable: bool,
    cjk_capable: bool,
}

struct SystemFontCatalog {
    faces: Vec<InstalledFace>,
}

const RESTRICTED: u16 = 0x0002;
const NO_SUBSETTING: u16 = 0x0100;
const BITMAP_ONLY: u16 = 0x0200;

fn has_sfnt_table(font: &FontRef, tag: &[u8; 4]) -> bool {
    font.data_for_tag(Tag::new(tag)).is_some()
}

fn inspect_installed_face(path: &Path, bytes: &[u8], face_index: u32) -> Option<InstalledFace> {
    let font = FontRef::from_index(bytes, face_index).ok()?;
    let ps = font
        .localized_strings(StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|s| s.chars().collect::<String>())
        .unwrap_or_default();

    let mut id1_families = Vec::new();
    for s in font.localized_strings(StringId::FAMILY_NAME) {
        let name: String = s.chars().collect();
        if !name.is_empty() && !id1_families.contains(&name) {
            id1_families.push(name);
        }
    }
    let mut families = id1_families.clone();
    for s in font.localized_strings(StringId::TYPOGRAPHIC_FAMILY_NAME) {
        let name: String = s.chars().collect();
        if !name.is_empty() && !families.contains(&name) {
            families.push(name);
        }
    }
    let mut full_names = Vec::new();
    for s in font.localized_strings(StringId::FULL_NAME) {
        let name: String = s.chars().collect();
        if !name.is_empty() && !full_names.contains(&name) {
            full_names.push(name);
        }
    }
    let sub: String = font
        .localized_strings(StringId::TYPOGRAPHIC_SUBFAMILY_NAME)
        .english_or_first()
        .or_else(|| {
            font.localized_strings(StringId::SUBFAMILY_NAME)
                .english_or_first()
        })
        .map(|s| s.chars().collect())
        .unwrap_or_default();

    let os2 = font.os2().ok();
    let weight_class = os2.as_ref().map_or(400, |o| o.us_weight_class());
    let width_class = os2.as_ref().map_or(5, |o| o.us_width_class());
    let fs_type = os2.as_ref().map_or(0, |o| o.fs_type());
    let panose = os2.as_ref().map(|o| o.panose_10());
    let head_mac_style = font.head().ok().map_or(0, |h| h.mac_style().bits());
    let subfamily_lower = sub.to_ascii_lowercase();
    let bold = weight_class >= 600 || (head_mac_style & 1 != 0) || subfamily_lower.contains("bold");
    let italic = (head_mac_style & 2 != 0)
        || subfamily_lower.contains("italic")
        || subfamily_lower.contains("oblique");
    let monospaced = font.post().ok().is_some_and(|p| p.is_fixed_pitch() != 0)
        || panose.is_some_and(|p| p[0] == 2 && p[3] == 9);
    // Panose family kind 2 (Latin Text): serif style 2..=10 are serif, 11..=15 sans-serif
    let serif = panose.is_some_and(|p| p[0] == 2 && (2..=10).contains(&p[1]))
        || families.iter().any(|f| {
            let fl = f.to_ascii_lowercase();
            fl.contains("times")
                || fl.contains("georgia")
                || fl.contains("cambria")
                || fl.contains("serif")
                || fl.contains("mingliu")
                || fl.contains("simsun")
                || fl.contains("kai")
        });

    let is_tt = has_sfnt_table(&font, b"glyf") && has_sfnt_table(&font, b"loca");
    let is_var = has_sfnt_table(&font, b"fvar");
    let is_color = [b"COLR", b"CBDT", b"sbix", b"SVG "]
        .iter()
        .any(|t| has_sfnt_table(&font, t));
    let embeddable = (fs_type & (RESTRICTED | NO_SUBSETTING | BITMAP_ONLY)) == 0;
    let truetype_embeddable = is_tt && !is_var && !is_color && embeddable;

    let charmap = font.charmap();
    let cjk_capable = charmap.map('中').is_some() && charmap.map('的').is_some();

    Some(InstalledFace {
        path: path.to_path_buf(),
        face_index,
        postscript_name: ps,
        id1_families,
        families,
        full_names,
        subfamily_lower,
        weight_class,
        width_class,
        bold,
        italic,
        monospaced,
        serif,
        truetype_embeddable,
        cjk_capable,
    })
}

impl SystemFontCatalog {
    fn load_windows_fonts() -> Self {
        let mut faces = Vec::new();
        let fonts_dir = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("WINDIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        let Ok(entries) = std::fs::read_dir(&fonts_dir) else {
            return Self { faces };
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|e| e.to_str()).is_some_and(|ext| {
                    matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "ttc" | "otf")
                })
            })
            .collect();
        paths.sort();

        for path in paths {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.len() < 12 || meta.len() > 64 * 1024 * 1024 {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let count = if bytes.starts_with(b"ttcf") {
                u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]).min(64)
            } else {
                1
            };
            for idx in 0..count {
                if let Some(face) = inspect_installed_face(&path, &bytes, idx) {
                    faces.push(face);
                }
            }
        }
        Self { faces }
    }

    /// Normalizes a PDF font name (e.g. `ABCDEF+Arial-BoldMT`, `TimesNewRomanPSMT`,
    /// `MicrosoftJhengHeiBold`, `PMingLiU`) into `(canonical_family_key, bold, italic)`.
    fn normalize_pdf_font_name(raw: &str) -> (String, bool, bool) {
        let (clean, _) = strip_subset_prefix(raw);
        let mut s = clean.trim().to_owned();
        let mut bold = false;
        let mut italic = false;

        // Split style suffix after ',' or '-'
        let lower = s.to_ascii_lowercase();
        if lower.contains("bold") || lower.contains("heavy") || lower.contains("semibold") {
            bold = true;
        }
        if lower.contains("italic") || lower.contains("oblique") {
            italic = true;
        }

        // Remove known style/PS/MT suffixes iteratively from the tail
        let base_part = s.split(',').next().unwrap_or(&s).to_owned();
        s = base_part;
        loop {
            let sl = s.to_ascii_lowercase();
            let stripped = [
                "-bolditalicmt",
                "-boldobliquemt",
                "-boldmt",
                "-italicmt",
                "-obliquemt",
                "ps-bolditalicmt",
                "ps-boldmt",
                "ps-italicmt",
                "psmt",
                "-bolditalic",
                "-boldoblique",
                "-bold",
                "-italic",
                "-oblique",
                "-regular",
                "-roman",
                "bolditalic",
                "boldoblique",
                "regular",
                "bold",
                "italic",
                "oblique",
                "mt",
            ]
            .iter()
            .find_map(|suf| {
                if sl.ends_with(suf) && s.len() > suf.len() {
                    Some(
                        s[..s.len() - suf.len()]
                            .trim_end_matches(['-', '_', ' '])
                            .to_owned(),
                    )
                } else {
                    None
                }
            });
            match stripped {
                Some(next) if !next.is_empty() => s = next,
                _ => break,
            }
        }
        if s.to_ascii_lowercase().ends_with("ps") && s.len() > 4 {
            let trimmed = &s[..s.len() - 2];
            if trimmed.eq_ignore_ascii_case("TimesNewRoman")
                || trimmed.eq_ignore_ascii_case("CourierNew")
            {
                s = trimmed.to_owned();
            }
        }

        let compact: String = s
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
            .collect::<String>()
            .to_ascii_lowercase();

        // Well-known PDF PostScript -> Windows family aliases
        let mapped = match compact.as_str() {
            "helvetica" | "helv" | "arial" => "arial",
            "times" | "timesroman" | "timesnewroman" | "tiro" => "timesnewroman",
            "courier" | "couriernew" | "cour" => "couriernew",
            "microsoftjhenghei" | "msjhenghei" | "微軟正黑體" => "microsoftjhenghei",
            "pmingliu" | "新細明體" => "pmingliu",
            "mingliu" | "細明體" => "mingliu",
            "dfkaisb" | "dfkaishusbestdbf" | "標楷體" => "dfkaisb",
            "simsun" | "宋體" | "宋体" => "simsun",
            "simhei" | "黑體" | "黑体" => "simhei",
            "microsoftyahei" | "微軟雅黑" | "微软雅黑" => "microsoftyahei",
            "segoeui" => "segoeui",
            "trebuchetms" => "trebuchetms",
            _ => compact.as_str(),
        };

        (mapped.to_owned(), bold, italic)
    }

    fn compact_key(s: &str) -> String {
        s.chars()
            .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
            .collect::<String>()
            .to_ascii_lowercase()
    }

    fn face_rank(
        f: &InstalledFace,
        target_family: &str,
        want_bold: bool,
        want_italic: bool,
    ) -> (u8, u8, u8, u8, u8) {
        let exact_style = u8::from(f.bold == want_bold && f.italic == want_italic);
        let normal_width = u8::from(
            f.width_class == 5
                && !f.subfamily_lower.contains("narrow")
                && !f.subfamily_lower.contains("condensed"),
        );
        let normal_weight = u8::from(
            (want_bold && (650..=750).contains(&f.weight_class))
                || (!want_bold && (350..=500).contains(&f.weight_class)),
        );
        let not_display_variant = u8::from(
            !f.subfamily_lower.contains("black")
                && !f.subfamily_lower.contains("light")
                && !f.subfamily_lower.contains("thin"),
        );
        let exact_id1 = u8::from(
            f.id1_families
                .iter()
                .any(|fam| Self::compact_key(fam) == target_family),
        );
        (
            exact_style,
            normal_width,
            exact_id1,
            normal_weight,
            not_display_variant,
        )
    }

    /// Matches a PDF font name + style flags against installed TrueType embeddable Windows fonts.
    fn match_exact(
        &self,
        pdf_name: &str,
        hint_bold: bool,
        hint_italic: bool,
    ) -> Option<&InstalledFace> {
        let (clean, _) = strip_subset_prefix(pdf_name);
        // 1. Exact PostScript name match (prefer embeddable TrueType)
        if let Some(face) = self
            .faces
            .iter()
            .find(|f| f.truetype_embeddable && f.postscript_name.eq_ignore_ascii_case(clean))
        {
            return Some(face);
        }

        // 2. Normalized family + (bold, italic), ranked so regular Arial beats Arial Narrow/Black
        let (norm_family, name_bold, name_italic) = Self::normalize_pdf_font_name(pdf_name);
        let want_bold = hint_bold || name_bold;
        let want_italic = hint_italic || name_italic;

        self.faces
            .iter()
            .filter(|f| {
                f.truetype_embeddable
                    && (f
                        .families
                        .iter()
                        .any(|fam| Self::compact_key(fam) == norm_family)
                        || f.full_names
                            .iter()
                            .any(|full| Self::compact_key(full) == norm_family))
            })
            .max_by_key(|f| Self::face_rank(f, &norm_family, want_bold, want_italic))
    }

    /// Finds the closest installed TrueType embeddable fallback matching traits.
    fn match_closest_fallback(
        &self,
        mono: bool,
        serif: bool,
        bold: bool,
        italic: bool,
        needs_cjk: bool,
    ) -> Option<&InstalledFace> {
        if needs_cjk {
            let preferred_cjk: &[&str] = if serif {
                &["pmingliu", "mingliu", "simsun", "microsoftjhenghei"]
            } else {
                &["microsoftjhenghei", "microsoftyahei", "simhei", "pmingliu"]
            };
            for target in preferred_cjk {
                if let Some(f) = self
                    .faces
                    .iter()
                    .filter(|f| {
                        f.truetype_embeddable
                            && f.cjk_capable
                            && f.families
                                .iter()
                                .any(|fam| Self::compact_key(fam) == *target)
                    })
                    .max_by_key(|f| Self::face_rank(f, target, bold, italic))
                {
                    return Some(f);
                }
            }
            return self
                .faces
                .iter()
                .find(|f| f.truetype_embeddable && f.cjk_capable);
        }

        let target_family = if mono {
            "couriernew"
        } else if serif {
            "timesnewroman"
        } else {
            "arial"
        };
        if let Some(face) = self
            .faces
            .iter()
            .filter(|f| {
                f.truetype_embeddable
                    && f.families
                        .iter()
                        .any(|fam| Self::compact_key(fam) == target_family)
            })
            .max_by_key(|f| Self::face_rank(f, target_family, bold, italic))
        {
            return Some(face);
        }
        // Any matching trait
        self.faces.iter().find(|f| {
            f.truetype_embeddable
                && f.monospaced == mono
                && f.serif == serif
                && f.bold == bold
                && f.italic == italic
        })
    }
}

// --------------------------------------------------------------------------- Line inspection & editability

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefusalReason {
    SignedDocument,
    ModifyForbidden,
    Type3Font,
    CustomEncodingNoToUnicode,
    VerticalWriting,
    InvisibleOrClipOnlyText,
    UnmappedGlyphs,
    DegenerateCoordinates,
    EmptyLine,
}

#[derive(Debug, Clone)]
struct InspectedChar {
    ch: char,
    origin: Point,
    quad: Quad,
    size: f32,
    argb: u32,
    flags: TextCharFlags,
    font_name: String,
    is_bold: bool,
    is_italic: bool,
    is_serif: bool,
    is_monospaced: bool,
}

#[derive(Debug, Clone)]
struct InspectedLine {
    text: String,
    wmode: WriteMode,
    bounds: Rect,
    chars: Vec<InspectedChar>,
    /// Matched PDF font resource from page `/Resources /Font`.
    font_resource: Option<PdfFontResource>,
    /// Dominant style from the first non-whitespace char.
    size: f32,
    color: Rgb,
    bold: bool,
    italic: bool,
    serif: bool,
    monospaced: bool,
    /// Unit vector along baseline in MuPDF page space.
    u_dir: Point,
    /// Up vector (from baseline toward ascender, capturing skew) in MuPDF page space.
    v_dir: Point,
}

fn argb_to_rgb(argb: u32) -> Rgb {
    Rgb {
        r: ((argb >> 16) & 0xFF) as u8,
        g: ((argb >> 8) & 0xFF) as u8,
        b: (argb & 0xFF) as u8,
    }
}

fn normalize_vec(dx: f32, dy: f32) -> Option<Point> {
    let len = dx.hypot(dy);
    if len > 1e-4 {
        Some(Point {
            x: dx / len,
            y: dy / len,
        })
    } else {
        None
    }
}

fn inspect_page_lines(doc: &MuPdf, page_index: i32) -> Result<Vec<InspectedLine>, mupdf::Error> {
    let page = doc.load_pdf_page(page_index)?;
    let font_resources = collect_page_fonts(&page);
    // Include USE_CID_FOR_UNKNOWN_UNICODE so unmapped characters set UNICODE_IS_CID instead of
    // silently disappearing, letting us detect missing-ToUnicode fonts cleanly.
    let tp = page.to_text_page(TextPageFlags::USE_CID_FOR_UNKNOWN_UNICODE)?;
    let mut lines = Vec::new();

    for block in tp.blocks() {
        if block.r#type() != TextBlockType::Text {
            continue;
        }
        for line in block.lines() {
            let mut text = String::new();
            let mut chars = Vec::new();
            for ch in line.chars() {
                let Some(c) = ch.char() else {
                    continue;
                };
                let (font_name, is_bold, is_italic, is_serif, is_monospaced) =
                    if let Some(f) = ch.font() {
                        (
                            f.name().to_owned(),
                            f.is_bold() || ch.flags().contains(TextCharFlags::BOLD),
                            f.is_italic(),
                            f.is_serif(),
                            f.is_monospaced(),
                        )
                    } else {
                        (
                            String::new(),
                            ch.flags().contains(TextCharFlags::BOLD),
                            false,
                            false,
                            false,
                        )
                    };
                text.push(c);
                chars.push(InspectedChar {
                    ch: c,
                    origin: ch.origin(),
                    quad: ch.quad(),
                    size: ch.size(),
                    argb: ch.argb(),
                    flags: ch.flags(),
                    font_name,
                    is_bold,
                    is_italic,
                    is_serif,
                    is_monospaced,
                });
            }
            if chars.is_empty() {
                continue;
            }
            let first = chars
                .iter()
                .find(|c| !c.ch.is_whitespace())
                .unwrap_or(&chars[0]);
            let last = chars
                .iter()
                .rev()
                .find(|c| !c.ch.is_whitespace())
                .unwrap_or(first);

            // Baseline direction: prefer first->last origin when distinct, else quad.ll->quad.lr
            let u_dir = normalize_vec(
                last.origin.x - first.origin.x,
                last.origin.y - first.origin.y,
            )
            .or_else(|| {
                normalize_vec(
                    first.quad.lr.x - first.quad.ll.x,
                    first.quad.lr.y - first.quad.ll.y,
                )
            })
            .unwrap_or(Point { x: 1.0, y: 0.0 });

            // Up direction (captures rotation + italic/skew shear from quad.ll -> quad.ul)
            let v_dir = normalize_vec(
                first.quad.ul.x - first.quad.ll.x,
                first.quad.ul.y - first.quad.ll.y,
            )
            .unwrap_or(Point {
                x: u_dir.y,
                y: -u_dir.x,
            });

            let font_resource = match_pdf_font_resource(&first.font_name, &font_resources).cloned();

            // MuPDF `fitz/font.c:806-817` defaults `is_serif = 1` for SFNT fonts whose OS/2
            // `sFamilyClass` is 0 ("No Classification", e.g. DroidSansFallback or stripped subsets).
            // Refine serif using PDF `/FontDescriptor /Flags` (bit 2 = Serif) and name hints first.
            let name_lower = font_resource
                .as_ref()
                .map(|r| r.clean_name.to_ascii_lowercase())
                .unwrap_or_else(|| first.font_name.to_ascii_lowercase());
            let serif = if name_lower.contains("sans")
                || name_lower.contains("arial")
                || name_lower.contains("helvetica")
                || name_lower.contains("calibri")
                || name_lower.contains("segoe")
                || name_lower.contains("gothic")
                || name_lower.contains("hei")
                || name_lower.contains("droid")
            {
                false
            } else if name_lower.contains("serif")
                || name_lower.contains("times")
                || name_lower.contains("georgia")
                || name_lower.contains("cambria")
                || name_lower.contains("ming")
                || name_lower.contains("song")
                || name_lower.contains("kai")
            {
                true
            } else if let Some(flags) = font_resource.as_ref().and_then(|r| r.fd_flags) {
                (flags & 2) != 0
            } else {
                first.is_serif
            };

            lines.push(InspectedLine {
                text,
                wmode: line.wmode(),
                bounds: line.bounds(),
                size: first.size,
                color: argb_to_rgb(first.argb),
                bold: first.is_bold,
                italic: first.is_italic,
                serif,
                monospaced: first.is_monospaced,
                u_dir,
                v_dir,
                font_resource,
                chars,
            });
        }
    }
    Ok(lines)
}

fn check_line_editability(
    worker_doc: &PdfDocument,
    line: &InspectedLine,
) -> Result<(), RefusalReason> {
    if worker_doc.is_signed() {
        return Err(RefusalReason::SignedDocument);
    }
    if !worker_doc.permissions().modify {
        return Err(RefusalReason::ModifyForbidden);
    }
    if line.text.trim().is_empty() {
        return Err(RefusalReason::EmptyLine);
    }
    if line.wmode == WriteMode::Vertical {
        return Err(RefusalReason::VerticalWriting);
    }
    let non_ws: Vec<&InspectedChar> = line
        .chars
        .iter()
        .filter(|c| !c.ch.is_whitespace())
        .collect();
    if non_ws.is_empty() {
        return Err(RefusalReason::EmptyLine);
    }
    // 3 Tr (invisible) and 7 Tr (clip-only) text:
    // - In stock MuPDF (`stext-device.c:1495-1506`), 7 Tr sets `FILLED | CLIPPED` with `argb == 0`,
    //   and 3 Tr sets `flags == 0` with `argb == 0`.
    // - With this repo's `mupdf-sys` build-time patch (#628/#638, see `tests/text.rs`), 7 Tr is
    //   emitted where shown via `fz_ignore_text`, so BOTH 3 Tr and 7 Tr report `flags == 0`
    //   (`!FILLED && !STROKED && !CLIPPED`) and `argb == 0`.
    // Refuse any line containing non-whitespace chars that are neither filled nor stroked (or
    // clipped with alpha == 0).
    if non_ws.iter().any(|c| {
        (c.flags.contains(TextCharFlags::CLIPPED) && (c.argb >> 24) == 0)
            || (!c.flags.contains(TextCharFlags::FILLED)
                && !c.flags.contains(TextCharFlags::STROKED))
    }) {
        return Err(RefusalReason::InvisibleOrClipOnlyText);
    }
    if let Some(res) = &line.font_resource {
        if res.subtype == "Type3" {
            return Err(RefusalReason::Type3Font);
        }
        if !res.has_to_unicode
            && matches!(
                res.encoding,
                EncodingKind::CustomDifferences {
                    has_non_standard_names: true,
                    ..
                }
            )
        {
            return Err(RefusalReason::CustomEncodingNoToUnicode);
        }
        // Symbolic font without ToUnicode and without standard WinAnsi/MacRoman
        if !res.has_to_unicode
            && res.fd_flags.is_some_and(|f| (f & 4) != 0 && (f & 32) == 0)
            && matches!(res.encoding, EncodingKind::Implicit)
        {
            return Err(RefusalReason::CustomEncodingNoToUnicode);
        }
    }
    if non_ws.iter().any(|c| {
        c.ch == '\u{FFFD}'
            || c.flags.contains(TextCharFlags::UNICODE_IS_CID)
            || c.flags.contains(TextCharFlags::UNICODE_IS_GID)
    }) {
        return Err(RefusalReason::UnmappedGlyphs);
    }
    // Malformed pages with huge MediaBox (e.g. 1e9 x 1e9) lose f32 precision and collapse glyph
    // quads to zero area; refuse cleanly rather than passing degenerate quads to MuPDF redaction.
    if !non_ws
        .iter()
        .any(|c| quad_is_valid_non_empty(&inset_quad_vertical(&c.quad, 0.15)))
    {
        return Err(RefusalReason::DegenerateCoordinates);
    }
    Ok(())
}

// --------------------------------------------------------------------------- B: can new text fit in original font?

fn is_winansi_printable(ch: char) -> bool {
    let u = u32::from(ch);
    (0x20..=0x7E).contains(&u)
        || (0xA0..=0xFF).contains(&u)
        || matches!(
            ch,
            '€' | '‚'
                | 'ƒ'
                | '„'
                | '…'
                | '†'
                | '‡'
                | 'ˆ'
                | '‰'
                | 'Š'
                | '‹'
                | 'Œ'
                | 'Ž'
                | '‘'
                | '’'
                | '“'
                | '”'
                | '•'
                | '–'
                | '—'
                | '˜'
                | '™'
                | 'š'
                | '›'
                | 'œ'
                | 'ž'
                | 'Ÿ'
        )
}

fn is_standard_14(clean_name: &str) -> bool {
    matches!(
        clean_name,
        "Helvetica"
            | "Helvetica-Bold"
            | "Helvetica-Oblique"
            | "Helvetica-BoldOblique"
            | "Times-Roman"
            | "Times-Bold"
            | "Times-Italic"
            | "Times-BoldItalic"
            | "Courier"
            | "Courier-Bold"
            | "Courier-Oblique"
            | "Courier-BoldOblique"
    )
}

/// Checks whether `new_text` can be written using ONLY the original PDF font resource (without
/// embedding a new font subset). Returns `Ok(encoded_pdf_operand)` (e.g. `(Text)` or `<0001...>`)
/// if possible, or `Err(reason)` otherwise.
fn try_encode_in_original_font(
    res: &PdfFontResource,
    new_text: &str,
) -> Result<String, &'static str> {
    if res.subtype == "Type3" {
        return Err("Type3 font");
    }

    // Case 1: Non-embedded Standard-14 (Type1) with WinAnsi or Implicit encoding
    if !res.embedded && is_standard_14(&res.clean_name) {
        if matches!(
            res.encoding,
            EncodingKind::WinAnsi | EncodingKind::Standard | EncodingKind::Implicit
        ) && new_text.chars().all(|c| c.is_ascii() && !c.is_control())
        {
            let escaped = new_text
                .replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)");
            return Ok(format!("({escaped})"));
        }
        return Err("character outside Standard-14 single-byte encoding");
    }

    // Case 2: Non-embedded Type0 with standard CJK Unicode CMap (e.g. UniCNS-UCS2-H).
    // If the font ALSO has a partial `/ToUnicode` table (like `benign/mixed-text-zh-en.pdf`),
    // MuPDF consults `/ToUnicode` during extraction, so any character missing from `/ToUnicode`
    // would become unsearchable. Only reuse directly when `/ToUnicode` is absent or covers all chars.
    if !res.embedded && res.subtype == "Type0" {
        if matches!(
            &res.encoding,
            EncodingKind::NamedCMap(name) if name.contains("UCS2-H") || name.contains("UTF16-H")
        ) && (!res.has_to_unicode
            || new_text
                .chars()
                .all(|c| res.unicode_to_code.contains_key(&c)))
        {
            let hex: String = new_text
                .encode_utf16()
                .map(|u| format!("{u:04X}"))
                .collect();
            return Ok(format!("<{hex}>"));
        }
        return Err("non-embedded Type0 with partial ToUnicode or non-Unicode CMap");
    }

    // Case 3: Embedded subset (Type0 Identity-H or TrueType subset)
    if res.is_subset {
        // Can only encode characters whose CID/code is ALREADY in this subset's /ToUnicode map!
        if res.unicode_to_code.is_empty() {
            return Err("subset font has no parseable /ToUnicode");
        }
        let mut hex = String::new();
        for ch in new_text.chars() {
            let Some(code_bytes) = res.unicode_to_code.get(&ch) else {
                return Err("glyph missing from embedded font subset");
            };
            for &b in code_bytes {
                hex.push_str(&format!("{b:02X}"));
            }
        }
        return Ok(format!("<{hex}>"));
    }

    // Case 4: Full embedded TrueType with WinAnsi encoding
    if res.embedded
        && res.subtype == "TrueType"
        && res.encoding == EncodingKind::WinAnsi
        && let Some(ttf) = &res.font_file2
        && let Ok(face) = FontRef::new(ttf)
    {
        let cmap = face.charmap();
        if new_text
            .chars()
            .all(|c| is_winansi_printable(c) && cmap.map(c).is_some())
        {
            let hex: String = new_text
                .chars()
                .map(|c| format!("{:02X}", c as u8))
                .collect();
            return Ok(format!("<{hex}>"));
        }
        return Err("character missing from full embedded TrueType or WinAnsi");
    }

    Err("font encoding/embedding does not allow safe direct reuse")
}

// --------------------------------------------------------------------------- ADR 0018 subset embedding

fn subset_tag(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    (0..6)
        .map(|i| char::from(b'A' + ((hash >> (i * 5)) % 26) as u8))
        .collect()
}

struct EmbeddedSubsetFont {
    font_obj: PdfObject,
    /// Hex-encoded CID string ready for `<...> Tj`.
    cid_hex: String,
    /// Total advance width in em.
    advance_em: f32,
    /// Raw subset TTF byte count.
    subset_bytes_len: usize,
}

/// Embeds a TrueType subset (ADR 0018: Type0 / CIDFontType2 / Identity-H + `/ToUnicode`) for `text`.
fn embed_truetype_subset(
    doc: &mut MuPdf,
    font_bytes: &[u8],
    face_index: u32,
    text: &str,
) -> Result<EmbeddedSubsetFont, &'static str> {
    let face = FontRef::from_index(font_bytes, face_index).map_err(|_| "not a font")?;
    if !(has_sfnt_table(&face, b"glyf") && has_sfnt_table(&face, b"loca")) {
        return Err("not TrueType glyf");
    }
    let charmap = face.charmap();
    let gmetrics = face.glyph_metrics(
        skrifa::instance::Size::unscaled(),
        skrifa::instance::LocationRef::default(),
    );
    let head = face.head().map_err(|_| "missing head")?;
    let per_em = f32::from(head.units_per_em());
    if per_em < 16.0 {
        return Err("invalid unitsPerEm");
    }

    let mut remapper = GlyphRemapper::new();
    remapper.remap(0);
    let mut char_cids: Vec<(char, u16, u16)> = Vec::new();
    let mut seen: BTreeMap<char, (u16, u16)> = BTreeMap::new();
    let mut advance_em = 0.0f32;

    for ch in text.chars() {
        let (cid, adv_1000) = if let Some(&entry) = seen.get(&ch) {
            entry
        } else {
            let gid = charmap
                .map(ch)
                .map(|g| g.to_u32())
                .filter(|&g| g != 0 && g <= u32::from(u16::MAX))
                .ok_or("missing glyph in font")? as u16;
            let cid = remapper.remap(gid);
            let adv = gmetrics
                .advance_width(GlyphId::new(u32::from(gid)))
                .unwrap_or(0.0);
            let adv_1000 = ((adv * 1000.0 / per_em).round() as i32).clamp(0, 5000) as u16;
            seen.insert(ch, (cid, adv_1000));
            (cid, adv_1000)
        };
        advance_em += f32::from(adv_1000) / 1000.0;
        char_cids.push((ch, cid, adv_1000));
    }

    let subset_bytes = subset(font_bytes, face_index, &remapper).map_err(|_| "subsetter failed")?;
    let subset_bytes_len = subset_bytes.len();
    let ps_raw: String = face
        .localized_strings(StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|s| s.chars().collect())
        .unwrap_or_default();
    let ps_clean: String = ps_raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(40)
        .collect();
    let base_name = if ps_clean.is_empty() {
        "EditedText".to_owned()
    } else {
        ps_clean
    };
    let base = format!("{}+{}", subset_tag(&subset_bytes), base_name);

    let fmetrics = face.metrics(
        skrifa::instance::Size::unscaled(),
        skrifa::instance::LocationRef::default(),
    );
    let to_1000 = |v: f32| (v * 1000.0 / per_em).round() as i32;
    let ascent = to_1000(fmetrics.ascent);
    let descent = to_1000(fmetrics.descent);
    let cap_height = to_1000(fmetrics.cap_height.unwrap_or(fmetrics.ascent));
    let bbox = [
        to_1000(f32::from(head.x_min())),
        to_1000(f32::from(head.y_min())),
        to_1000(f32::from(head.x_max())),
        to_1000(f32::from(head.y_max())),
    ];

    let len_dict = parse_obj(doc, &format!("<< /Length1 {} >>", subset_bytes.len()));
    let file_obj = doc
        .add_stream(
            &Buffer::from_bytes(&subset_bytes).map_err(|_| "buf")?,
            Some(&len_dict),
            false,
        )
        .map_err(|_| "add_stream")?;
    let desc_obj = parse_obj(
        doc,
        &format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags 4 /FontBBox [{} {} {} {}] \
             /ItalicAngle 0 /Ascent {ascent} /Descent {descent} /CapHeight {cap_height} \
             /StemV 80 /FontFile2 {} 0 R >>",
            bbox[0],
            bbox[1],
            bbox[2],
            bbox[3],
            file_obj.as_indirect().map_err(|_| "indirect")?,
        ),
    );
    let desc_obj = doc.add_object(&desc_obj).map_err(|_| "add desc")?;

    let mut widths_map: BTreeMap<u16, u16> = BTreeMap::new();
    let mut tounicode_map: BTreeMap<u16, char> = BTreeMap::new();
    for &(ch, cid, adv) in &char_cids {
        widths_map.insert(cid, adv);
        tounicode_map.entry(cid).or_insert(ch);
    }
    let widths_str: String = widths_map
        .iter()
        .map(|(cid, w)| format!("{cid} [{w}] "))
        .collect();
    let cid_font = parse_obj(
        doc,
        &format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base} \
             /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
             /FontDescriptor {} 0 R /CIDToGIDMap /Identity /DW 1000 /W [{widths_str}] >>",
            desc_obj.as_indirect().map_err(|_| "indirect")?,
        ),
    );
    let cid_font = doc.add_object(&cid_font).map_err(|_| "add cid")?;

    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def \
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def \
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    cmap.push_str(&format!("{} beginbfchar\n", tounicode_map.len()));
    for (cid, ch) in &tounicode_map {
        let mut units = [0u16; 2];
        let u16_hex: String = ch
            .encode_utf16(&mut units)
            .iter()
            .map(|u| format!("{u:04X}"))
            .collect();
        cmap.push_str(&format!("<{cid:04X}> <{u16_hex}>\n"));
    }
    cmap.push_str("endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end\n");
    let tu_obj = doc
        .add_stream(
            &Buffer::from_bytes(cmap.as_bytes()).map_err(|_| "tu buf")?,
            None,
            false,
        )
        .map_err(|_| "add tu")?;

    let type0 = parse_obj(
        doc,
        &format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H \
             /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>",
            cid_font.as_indirect().map_err(|_| "indirect")?,
            tu_obj.as_indirect().map_err(|_| "indirect")?,
        ),
    );
    let font_obj = doc.add_object(&type0).map_err(|_| "add type0")?;

    let cid_hex: String = char_cids
        .iter()
        .map(|(_, cid, _)| format!("{cid:04X}"))
        .collect();

    Ok(EmbeddedSubsetFont {
        font_obj,
        cid_hex: format!("<{cid_hex}>"),
        advance_em,
        subset_bytes_len,
    })
}

// --------------------------------------------------------------------------- C: edit one line

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontStrategy {
    /// Only use the existing PDF font object; refuse if any char is missing.
    OriginalOnlyOrRefuse,
    /// Use existing PDF font if all chars fit; else subset the same installed font; else refuse.
    SameFontOrRefuse,
    /// Use existing PDF font if all chars fit; else subset same installed font; else closest
    /// installed fallback (or bundled Droid Sans Fallback for CJK).
    BestAvailableFallback,
}

#[derive(Debug, Clone)]
struct EditOutcome {
    pdf_bytes: Vec<u8>,
    font_used: String,
    substituted: bool,
    subset_bytes: usize,
    new_width_pt: f32,
}

/// Insets `quad` vertically along its `ll->ul` / `lr->ur` edges by `frac` on both top and bottom.
/// MuPDF's `pdf_redact_text_filter` (`pdf-clean.c:552-557`) shrinks candidate glyph bboxes by 10%
/// but uses the raw annotation quad as-is; full-height `TextChar::quad()` (ascender..descender,
/// ~1.16x font size) can reach into tightly-spaced adjacent lines (e.g. 1.125x leading).
/// A 15% vertical inset keeps the quad well inside the target glyph's central 80% while sparing
/// tightly-spaced lines above and below.
fn inset_quad_vertical(q: &Quad, frac: f32) -> Quad {
    let lerp = |a: Point, b: Point, t: f32| Point {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
    };
    Quad {
        ul: lerp(q.ul, q.ll, frac),
        ur: lerp(q.ur, q.lr, frac),
        ll: lerp(q.ll, q.ul, frac),
        lr: lerp(q.lr, q.ur, frac),
    }
}

fn quad_is_valid_non_empty(q: &Quad) -> bool {
    let pts = [q.ul, q.ur, q.ll, q.lr];
    if !pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        return false;
    }
    let min_x = pts.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let max_x = pts.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let min_y = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_y = pts.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    (max_x - min_x) > 0.05 && (max_y - min_y) > 0.05
}

/// Replaces characters `char_range` (in `line.chars`) with `new_text` on `page_index`:
/// 1. Removes old glyphs via `pdf_redact_page` using vertically-inset per-character `QuadPoints`
///    (`black_boxes: false`, `image_method: None`, `line_art: None`, `text: Remove`).
/// 2. Draws `new_text` at the original baseline origin, orientation/skew matrix, size, and colour,
///    wrapping existing `/Contents` in `q ... Q` (following `page_overlay.rs`).
#[allow(clippy::too_many_arguments)]
fn edit_line_range(
    pdf_bytes: &[u8],
    password: Option<&str>,
    page_index: u32,
    line: &InspectedLine,
    char_range: std::ops::Range<usize>,
    new_text: &str,
    catalog: &SystemFontCatalog,
    strategy: FontStrategy,
) -> Result<EditOutcome, String> {
    let worker_doc = PdfDocument::open(pdf_bytes, password).map_err(|e| format!("open: {e}"))?;
    check_line_editability(&worker_doc, line).map_err(|r| format!("refused: {r:?}"))?;

    let target_chars = &line.chars[char_range.clone()];
    let target_quads: Vec<Quad> = target_chars
        .iter()
        .filter(|c| !c.ch.is_whitespace())
        .map(|c| inset_quad_vertical(&c.quad, 0.15))
        .filter(quad_is_valid_non_empty)
        .collect();
    if target_quads.is_empty() {
        return Err("no non-whitespace glyphs in target range".to_owned());
    }
    let anchor_char = target_chars
        .iter()
        .find(|c| !c.ch.is_whitespace())
        .unwrap_or(&target_chars[0]);

    let mut mupdf_doc = MuPdf::from_bytes(pdf_bytes).map_err(|e| format!("mupdf open: {e}"))?;
    if mupdf_doc.needs_password().unwrap_or(false) {
        let pw = password.ok_or_else(|| "password required".to_owned())?;
        mupdf_doc
            .authenticate(pw)
            .map_err(|e| format!("auth: {e}"))?;
    }

    // Determine font & encoded text BEFORE mutating page contents so failures leave doc clean.
    let needs_cjk = new_text.chars().any(|c| u32::from(c) > 0x2E7F);
    let font_used: String;
    let mut substituted = false;
    let mut subset_bytes = 0usize;
    let new_width_pt: f32;

    enum ChosenFont {
        ExistingResource {
            key: String,
            operand: String,
        },
        NewSubset {
            font_obj: PdfObject,
            operand: String,
        },
    }

    let chosen = if let Some(res) = &line.font_resource
        && let Ok(operand) = try_encode_in_original_font(res, new_text)
        && !res.resource_key.is_empty()
        // Only reuse directly if the resource is directly on the page's /Resources /Font
        && mupdf_doc
            .load_pdf_page(page_index as i32)
            .ok()
            .and_then(|p| p.object().get_dict("Resources").ok().flatten())
            .and_then(|r| r.get_dict("Font").ok().flatten())
            .and_then(|f| f.get_dict(res.resource_key.as_str()).ok().flatten())
            .is_some()
    {
        font_used = format!("original:{}", res.clean_name);
        // Approximate width for Standard-14 (0.55 em per char)
        new_width_pt = new_text.chars().count() as f32 * line.size * 0.55;
        ChosenFont::ExistingResource {
            key: res.resource_key.clone(),
            operand,
        }
    } else {
        if strategy == FontStrategy::OriginalOnlyOrRefuse {
            return Err("refused: replacement glyphs missing from original PDF font".to_owned());
        }
        let pdf_font_name = line
            .font_resource
            .as_ref()
            .map(|r| r.clean_name.as_str())
            .unwrap_or_else(|| anchor_char.font_name.as_str());

        let exact = catalog.match_exact(pdf_font_name, line.bold, line.italic);
        let embedded_exact = exact.and_then(|face| {
            let bytes = std::fs::read(&face.path).ok()?;
            let sub =
                embed_truetype_subset(&mut mupdf_doc, &bytes, face.face_index, new_text).ok()?;
            Some((face, sub))
        });

        if let Some((face, sub)) = embedded_exact {
            font_used = format!("installed-exact:{}", face.postscript_name);
            subset_bytes = sub.subset_bytes_len;
            new_width_pt = sub.advance_em * line.size;
            ChosenFont::NewSubset {
                font_obj: sub.font_obj,
                operand: sub.cid_hex,
            }
        } else {
            if strategy == FontStrategy::SameFontOrRefuse {
                return Err(format!(
                    "refused: exact installed font not found or lacks glyphs for '{pdf_font_name}'"
                ));
            }
            substituted = true;
            let fallback = catalog
                .match_closest_fallback(
                    line.monospaced,
                    line.serif,
                    line.bold,
                    line.italic,
                    needs_cjk,
                )
                .and_then(|face| {
                    let bytes = std::fs::read(&face.path).ok()?;
                    let sub =
                        embed_truetype_subset(&mut mupdf_doc, &bytes, face.face_index, new_text)
                            .ok()?;
                    Some((format!("fallback:{}", face.postscript_name), sub))
                })
                .or_else(|| {
                    let sub =
                        embed_truetype_subset(&mut mupdf_doc, bundled_droid(), 0, new_text).ok()?;
                    Some(("fallback:DroidSansFallback(bundled)".to_owned(), sub))
                })
                .ok_or_else(|| "no fallback font could encode replacement text".to_owned())?;

            font_used = fallback.0;
            subset_bytes = fallback.1.subset_bytes_len;
            new_width_pt = fallback.1.advance_em * line.size;
            ChosenFont::NewSubset {
                font_obj: fallback.1.font_obj,
                operand: fallback.1.cid_hex,
            }
        }
    };

    // Step 1: Remove ONLY the target characters via per-char QuadPoints redaction.
    {
        let mut page = mupdf_doc
            .load_pdf_page(page_index as i32)
            .map_err(|e| format!("load page: {e}"))?;
        page.add_redact_annotation(target_quads)
            .map_err(|e| format!("add redact: {e}"))?;
        let opts = PdfRedactOptions {
            black_boxes: false,
            image_method: PdfRedactImageMethod::None,
            line_art: PdfRedactLineArtMethod::None,
            text: PdfRedactTextMethod::Remove,
        };
        page.redact_with_options(opts)
            .map_err(|e| format!("redact: {e}"))?;
    }

    // Step 2: Compute PDF-space text matrix from page-space baseline origin + (u_dir, v_dir).
    let mut page_obj = mupdf_doc
        .load_pdf_page(page_index as i32)
        .map_err(|e| format!("reload page: {e}"))?
        .object();
    let inv = page_obj
        .page_ctm()
        .map_err(|e| format!("page_ctm: {e}"))?
        .invert()
        .ok_or_else(|| "non-invertible page_ctm".to_owned())?;

    let o = anchor_char.origin;
    let p0 = inv.transform_xy(o.x, o.y);
    let pu = inv.transform_xy(o.x + line.u_dir.x, o.y + line.u_dir.y);
    let pv = inv.transform_xy(o.x + line.v_dir.x, o.y + line.v_dir.y);
    let (ux, uy) = (pu.0 - p0.0, pu.1 - p0.1);
    let (vx, vy) = (pv.0 - p0.0, pv.1 - p0.1);

    // Ensure page has its own /Resources and /Font dictionary.
    let mut resources = match page_obj.get_dict("Resources").map_err(|e| e.to_string())? {
        Some(res) if res.is_dict().unwrap_or(false) => res,
        _ => {
            let copied = match page_obj
                .get_dict_inheritable("Resources")
                .map_err(|e| e.to_string())?
            {
                Some(inh) => inh.copy_dict().map_err(|e| e.to_string())?,
                None => mupdf_doc.new_dict().map_err(|e| e.to_string())?,
            };
            page_obj
                .dict_put("Resources", copied)
                .map_err(|e| e.to_string())?;
            page_obj
                .get_dict("Resources")
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "missing resources".to_owned())?
        }
    };

    let (font_res_key, operand) = match chosen {
        ChosenFont::ExistingResource { key, operand } => (key, operand),
        ChosenFont::NewSubset { font_obj, operand } => {
            let mut fonts_dict = match resources.get_dict("Font").map_err(|e| e.to_string())? {
                Some(f) if f.is_dict().unwrap_or(false) => f,
                _ => {
                    let d = mupdf_doc.new_dict().map_err(|e| e.to_string())?;
                    resources.dict_put("Font", d).map_err(|e| e.to_string())?;
                    resources
                        .get_dict("Font")
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| "missing font dict".to_owned())?
                }
            };
            let mut idx = 1usize;
            let key = loop {
                let candidate = format!("PdfEditF{idx}");
                if fonts_dict
                    .get_dict(candidate.as_str())
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    break candidate;
                }
                idx += 1;
            };
            fonts_dict
                .dict_put(key.as_str(), font_obj)
                .map_err(|e| e.to_string())?;
            (key, operand)
        }
    };

    // Step 3: Wrap existing contents in `q ... Q` and append the new text stream.
    let before = mupdf_doc
        .add_stream(
            &Buffer::from_bytes(b"q\n").map_err(|e| e.to_string())?,
            None,
            false,
        )
        .map_err(|e| e.to_string())?;
    let r = f32::from(line.color.r) / 255.0;
    let g = f32::from(line.color.g) / 255.0;
    let b = f32::from(line.color.b) / 255.0;
    let draw = format!(
        "\nQ\nq\nBT\n/{font_res_key} {:.3} Tf\n{r:.4} {g:.4} {b:.4} rg\n\
         {ux:.5} {uy:.5} {vx:.5} {vy:.5} {:.3} {:.3} Tm\n{operand} Tj\nET\nQ\n",
        line.size, p0.0, p0.1,
    );
    let after = mupdf_doc
        .add_stream(
            &Buffer::from_bytes(draw.as_bytes()).map_err(|e| e.to_string())?,
            None,
            false,
        )
        .map_err(|e| e.to_string())?;

    let mut contents = mupdf_doc.new_array().map_err(|e| e.to_string())?;
    contents.array_push(before).map_err(|e| e.to_string())?;
    match page_obj.get_dict("Contents").map_err(|e| e.to_string())? {
        Some(existing) if existing.is_array().unwrap_or(false) => {
            for part in existing.array_iter().map_err(|e| e.to_string())? {
                contents
                    .array_push(part.map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            }
        }
        Some(existing) => contents.array_push(existing).map_err(|e| e.to_string())?,
        None => {}
    }
    contents.array_push(after).map_err(|e| e.to_string())?;
    page_obj
        .dict_put("Contents", contents)
        .map_err(|e| e.to_string())?;

    let saved = save_mupdf(&mupdf_doc, true);
    Ok(EditOutcome {
        pdf_bytes: saved,
        font_used,
        substituted,
        subset_bytes,
        new_width_pt,
    })
}

// --------------------------------------------------------------------------- D: Scanned page Option 1

fn median_u8(values: &mut [u8]) -> u8 {
    if values.is_empty() {
        return 255;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

/// Edits a line on a scanned page (Option 1):
/// - Finds the scan image XObject and its page-space CTM;
/// - Maps `line_rect` (page space) into pixel coordinates;
/// - Samples surrounding border pixels to determine local paper background colour;
/// - Fills the text pixel box with that background colour (and re-encodes JPEG if `/DCTDecode`,
///   or updates raw/Flate pixels for lossless/1-bit scans);
/// - Removes any pre-existing invisible `3 Tr` text in `line_rect`;
/// - Writes `new_text` visibly into page content in a subset of Droid Sans Fallback (or matched
///   default sans) using sampled ink colour and font size derived from `line_rect`.
type ScanEditOutput = (Vec<u8>, [u8; 3], [u8; 3]);

fn edit_scanned_page_option1(
    pdf_bytes: &[u8],
    line_rect: Rect,
    new_text: &str,
    jpeg_quality: u8,
) -> Result<ScanEditOutput, String> {
    let mut doc = MuPdf::from_bytes(pdf_bytes).map_err(|e| e.to_string())?;

    // 1. Remove any pre-existing invisible (3 Tr) text layer in the edited area first.
    {
        let mut page = doc.load_pdf_page(0).map_err(|e| e.to_string())?;
        page.add_redact_annotation(line_rect)
            .map_err(|e| e.to_string())?;
        let opts = PdfRedactOptions {
            black_boxes: false,
            image_method: PdfRedactImageMethod::None,
            line_art: PdfRedactLineArtMethod::None,
            text: PdfRedactTextMethod::Remove,
        };
        page.redact_with_options(opts).map_err(|e| e.to_string())?;
    }

    // 2. Find the main image block's CTM in page space.
    let img_ctm = {
        let page = doc.load_pdf_page(0).map_err(|e| e.to_string())?;
        let tp = page
            .to_text_page(TextPageFlags::PRESERVE_IMAGES)
            .map_err(|e| e.to_string())?;
        tp.blocks()
            .filter(|b| b.r#type() == TextBlockType::Image)
            .filter_map(|b| b.ctm())
            .next()
            .ok_or_else(|| "no image block found on scanned page".to_owned())?
    };

    // 3. Locate the image XObject in `/Resources /XObject`.
    let page_obj = doc.load_pdf_page(0).map_err(|e| e.to_string())?.object();
    let res = page_obj
        .get_dict("Resources")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no resources".to_owned())?;
    let xobjs = res
        .get_dict("XObject")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no xobjects".to_owned())?;
    let mut img_obj = None;
    for item in xobjs.dict_iter().map_err(|e| e.to_string())? {
        let (_, val) = item.map_err(|e| e.to_string())?;
        let is_img = val
            .get_dict("Subtype")
            .ok()
            .flatten()
            .and_then(|s| s.as_name().ok())
            .is_some_and(|n| n == b"Image");
        if is_img {
            img_obj = Some(val);
            break;
        }
    }
    let mut img_obj = img_obj.ok_or_else(|| "no image XObject".to_owned())?;

    let w = img_obj
        .get_dict("Width")
        .map_err(|e| e.to_string())?
        .ok_or("no Width")?
        .as_int()
        .map_err(|e| e.to_string())? as usize;
    let h = img_obj
        .get_dict("Height")
        .map_err(|e| e.to_string())?
        .ok_or("no Height")?
        .as_int()
        .map_err(|e| e.to_string())? as usize;
    let bpc = img_obj
        .get_dict("BitsPerComponent")
        .ok()
        .flatten()
        .and_then(|v| v.as_int().ok())
        .unwrap_or(8);
    let cs_name = img_obj
        .get_dict("ColorSpace")
        .ok()
        .flatten()
        .and_then(|v| v.as_name().ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_else(|| "DeviceRGB".to_owned());
    let is_jpeg = img_obj
        .get_dict("Filter")
        .ok()
        .flatten()
        .and_then(|v| v.as_name().ok())
        .is_some_and(|n| n == b"DCTDecode");

    let channels: usize = match cs_name.as_str() {
        "DeviceGray" => 1,
        "DeviceRGB" => 3,
        other => return Err(format!("unsupported scan ColorSpace: {other}")),
    };

    // Map line_rect (page space) -> image pixel space [0..w] x [0..h] (top-down rows).
    // Note: MuPDF's stext image block `ctm` maps unit square [0,1]x[0,1] (with y=0 top or bottom
    // depending on CTM sign) to page space. Let's inspect its transformed corners directly!
    let inv_img = img_ctm.invert().ok_or("non-invertible image ctm")?;
    let corners = [
        inv_img.transform_xy(line_rect.x0, line_rect.y0),
        inv_img.transform_xy(line_rect.x1, line_rect.y0),
        inv_img.transform_xy(line_rect.x0, line_rect.y1),
        inv_img.transform_xy(line_rect.x1, line_rect.y1),
    ];
    let ux0 = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let ux1 = corners
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let uy0 = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let uy1 = corners
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max);

    // Check whether unit y=0 is at page bottom (standard PDF upright image) or top.
    let p_top = img_ctm.transform_xy(0.0, 0.0);
    let p_bot = img_ctm.transform_xy(0.0, 1.0);
    let y_flipped = p_bot.1 < p_top.1;

    let px0 = ((ux0 * w as f32).floor() as isize).clamp(0, w as isize) as usize;
    let px1 = ((ux1 * w as f32).ceil() as isize).clamp(0, w as isize) as usize;
    let (py0, py1) = if y_flipped {
        (
            (((1.0 - uy1) * h as f32).floor() as isize).clamp(0, h as isize) as usize,
            (((1.0 - uy0) * h as f32).ceil() as isize).clamp(0, h as isize) as usize,
        )
    } else {
        (
            ((uy0 * h as f32).floor() as isize).clamp(0, h as isize) as usize,
            ((uy1 * h as f32).ceil() as isize).clamp(0, h as isize) as usize,
        )
    };

    // Decode raw stream into 8-bit unpacked pixels
    let decoded = img_obj.read_stream().map_err(|e| e.to_string())?;
    let mut pixels: Vec<u8> = if bpc == 1 && channels == 1 {
        let row_bytes = w.div_ceil(8);
        let mut unpacked = vec![255u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let byte = decoded.get(y * row_bytes + x / 8).copied().unwrap_or(255);
                let bit = (byte >> (7 - (x % 8))) & 1;
                unpacked[y * w + x] = if bit != 0 { 255 } else { 0 };
            }
        }
        unpacked
    } else if bpc == 8 && decoded.len() == w * h * channels {
        decoded
    } else {
        return Err(format!(
            "unsupported scan pixel format: {w}x{h} bpc={bpc} ch={channels} decoded={}",
            decoded.len()
        ));
    };

    // Sample border ring (4 px outside the text box) for paper background colour
    let pad = 4usize;
    let bx0 = px0.saturating_sub(pad);
    let bx1 = (px1 + pad).min(w);
    let by0 = py0.saturating_sub(pad);
    let by1 = (py1 + pad).min(h);

    let mut ch_samples: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for y in by0..by1 {
        for x in bx0..bx1 {
            if x >= px0 && x < px1 && y >= py0 && y < py1 {
                continue;
            }
            let at = (y * w + x) * channels;
            for c in 0..channels {
                ch_samples[c].push(pixels[at + c]);
            }
        }
    }
    let bg = if channels == 1 {
        let g = median_u8(&mut ch_samples[0]);
        [g, g, g]
    } else {
        [
            median_u8(&mut ch_samples[0]),
            median_u8(&mut ch_samples[1]),
            median_u8(&mut ch_samples[2]),
        ]
    };

    // Sample darkest 15% of pixels inside the box for ink colour
    let mut dark_pixels: Vec<(u32, [u8; 3])> = Vec::new();
    for y in py0..py1 {
        for x in px0..px1 {
            let at = (y * w + x) * channels;
            let rgb = if channels == 1 {
                let g = pixels[at];
                [g, g, g]
            } else {
                [pixels[at], pixels[at + 1], pixels[at + 2]]
            };
            let luma = u32::from(rgb[0]) + u32::from(rgb[1]) + u32::from(rgb[2]);
            dark_pixels.push((luma, rgb));
        }
    }
    dark_pixels.sort_by_key(|(luma, _)| *luma);
    let ink = if dark_pixels.is_empty() {
        [0, 0, 0]
    } else {
        let take = (dark_pixels.len() / 20).max(1);
        let mut rs: Vec<u8> = dark_pixels[..take].iter().map(|(_, c)| c[0]).collect();
        let mut gs: Vec<u8> = dark_pixels[..take].iter().map(|(_, c)| c[1]).collect();
        let mut bs: Vec<u8> = dark_pixels[..take].iter().map(|(_, c)| c[2]).collect();
        [median_u8(&mut rs), median_u8(&mut gs), median_u8(&mut bs)]
    };

    // Fill the old text region with sampled paper background colour
    for y in py0..py1 {
        for x in px0..px1 {
            let at = (y * w + x) * channels;
            pixels[at..at + channels].copy_from_slice(&bg[..channels]);
        }
    }

    // Write modified image pixels back
    if is_jpeg {
        let mut jpeg = Vec::new();
        let mut enc = Encoder::new(&mut jpeg, jpeg_quality);
        enc.set_optimized_huffman_tables(true);
        enc.set_chroma_subsampling_method(ChromaSubsamplingMethod::Average);
        let color_type = if channels == 3 {
            ColorType::Rgb
        } else {
            ColorType::Luma
        };
        enc.encode(&pixels, w as u16, h as u16, color_type)
            .map_err(|e| format!("jpeg encode: {e}"))?;
        img_obj
            .dict_put(
                "Filter",
                PdfObject::new_name("DCTDecode").map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        img_obj
            .dict_delete("DecodeParms")
            .map_err(|e| e.to_string())?;
        img_obj
            .dict_put(
                "BitsPerComponent",
                PdfObject::new_int(8).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        img_obj
            .write_raw_stream_buffer(&Buffer::from_bytes(&jpeg).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    } else if bpc == 1 && channels == 1 {
        // Pack back to 1-bit grayscale
        let row_bytes = w.div_ceil(8);
        let mut packed = vec![0u8; row_bytes * h];
        for y in 0..h {
            for x in 0..w {
                if pixels[y * w + x] >= 128 {
                    packed[y * row_bytes + x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        img_obj.dict_delete("Filter").map_err(|e| e.to_string())?;
        img_obj
            .dict_delete("DecodeParms")
            .map_err(|e| e.to_string())?;
        img_obj
            .write_stream_buffer(&Buffer::from_bytes(&packed).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    } else {
        img_obj.dict_delete("Filter").map_err(|e| e.to_string())?;
        img_obj
            .dict_delete("DecodeParms")
            .map_err(|e| e.to_string())?;
        img_obj
            .write_stream_buffer(&Buffer::from_bytes(&pixels).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }

    // 4. Write new visible text into page content at the OCR line's baseline & height
    let sub = embed_truetype_subset(&mut doc, bundled_droid(), 0, new_text)?;
    let mut page_obj = doc.load_pdf_page(0).map_err(|e| e.to_string())?.object();
    let inv = page_obj
        .page_ctm()
        .map_err(|e| e.to_string())?
        .invert()
        .ok_or("non-invertible page_ctm")?;

    let box_height = (line_rect.y1 - line_rect.y0).max(6.0);
    let font_size = box_height * 0.85;
    // Place baseline ~12% above bottom of OCR box
    let baseline_page = Point {
        x: line_rect.x0,
        y: line_rect.y1 - box_height * 0.12,
    };
    let p0 = inv.transform_xy(baseline_page.x, baseline_page.y);

    let mut resources = page_obj
        .get_dict("Resources")
        .map_err(|e| e.to_string())?
        .ok_or("no resources")?;
    let mut fonts = match resources.get_dict("Font").map_err(|e| e.to_string())? {
        Some(f) if f.is_dict().unwrap_or(false) => f,
        _ => {
            let d = doc.new_dict().map_err(|e| e.to_string())?;
            resources.dict_put("Font", d).map_err(|e| e.to_string())?;
            resources
                .get_dict("Font")
                .map_err(|e| e.to_string())?
                .ok_or("no font dict")?
        }
    };
    fonts
        .dict_put("PdfEditScanF1", sub.font_obj)
        .map_err(|e| e.to_string())?;

    let before = doc
        .add_stream(
            &Buffer::from_bytes(b"q\n").map_err(|e| e.to_string())?,
            None,
            false,
        )
        .map_err(|e| e.to_string())?;
    let draw = format!(
        "\nQ\nq\nBT\n/PdfEditScanF1 {font_size:.3} Tf\n{:.4} {:.4} {:.4} rg\n\
         1 0 0 1 {:.3} {:.3} Tm\n{} Tj\nET\nQ\n",
        f32::from(ink[0]) / 255.0,
        f32::from(ink[1]) / 255.0,
        f32::from(ink[2]) / 255.0,
        p0.0,
        p0.1,
        sub.cid_hex,
    );
    let after = doc
        .add_stream(
            &Buffer::from_bytes(draw.as_bytes()).map_err(|e| e.to_string())?,
            None,
            false,
        )
        .map_err(|e| e.to_string())?;

    let mut contents = doc.new_array().map_err(|e| e.to_string())?;
    contents.array_push(before).map_err(|e| e.to_string())?;
    match page_obj.get_dict("Contents").map_err(|e| e.to_string())? {
        Some(existing) if existing.is_array().unwrap_or(false) => {
            for part in existing.array_iter().map_err(|e| e.to_string())? {
                contents
                    .array_push(part.map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            }
        }
        Some(existing) => contents.array_push(existing).map_err(|e| e.to_string())?,
        None => {}
    }
    contents.array_push(after).map_err(|e| e.to_string())?;
    page_obj
        .dict_put("Contents", contents)
        .map_err(|e| e.to_string())?;

    let mut opts = PdfWriteOptions::default();
    opts.set_garbage(true);
    opts.set_compress(true);
    opts.set_compress_images(true);
    let mut out = Vec::new();
    doc.write_to_with_options(&mut out, opts)
        .map_err(|e| e.to_string())?;
    Ok((out, bg, ink))
}

// --------------------------------------------------------------------------- Synthetic sample generators

/// Standard 14 fonts in varied sizes, styles, and colours (including Tm-scaled font size).
fn sample_std14() -> Vec<u8> {
    let content = "\
BT /F1 18 Tf 0 0 0 rg 72 720 Td (Helvetica 18pt Black Line) Tj ET\n\
BT /F2 16 Tf 0.8 0.1 0.1 rg 72 680 Td (Helvetica Bold Red Line) Tj ET\n\
BT /F3 14 Tf 0.1 0.2 0.7 rg 72 640 Td (Times Italic Blue Line) Tj ET\n\
BT /F4 12 Tf 0.2 0.5 0.2 rg 72 600 Td (Courier Monospace Green) Tj ET\n\
BT /F1 1 Tf 0 0 0 rg 24 0 0 24 72 550 Tm (Matrix Scaled 24pt) Tj ET\n";
    common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R /F2 6 0 R /F3 7 0 R /F4 8 0 R >> >> >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Italic /Encoding /WinAnsiEncoding >>"
            .into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".into(),
    ])
}

/// Generates a PDF whose text is set in an embedded TrueType/Type0 subset (e.g. `ABCDEF+ArialMT`
/// or `ABCDEF+MicrosoftJhengHei` or `ABCDEF+DroidSansFallback`).
fn sample_embedded_subset(
    font_bytes: &[u8],
    face_index: u32,
    lines: &[(&str, f32, Rgb)],
) -> Vec<u8> {
    let mut doc = MuPdf::from_bytes(&common::one_page("")).expect("blank");
    let all_text: String = lines.iter().map(|(t, _, _)| *t).collect();
    let sub = embed_truetype_subset(&mut doc, font_bytes, face_index, &all_text).expect("subset");

    // Re-encode each line using the single subset's ToUnicode / CIDs: easiest is to embed per line
    // or map through the single font. Here we embed one subset per line so each has exact CIDs.
    drop(sub);
    let mut font_entries = Vec::new();
    let mut ops = String::new();
    for (idx, (text, size, color)) in lines.iter().enumerate() {
        let s = embed_truetype_subset(&mut doc, font_bytes, face_index, text).expect("sub");
        let key = format!("F{}", idx + 1);
        font_entries.push(format!(
            "/{key} {} 0 R",
            s.font_obj.as_indirect().expect("indirect")
        ));
        let y = 720.0 - idx as f32 * 40.0;
        ops.push_str(&format!(
            "BT /{key} {size:.2} Tf {:.4} {:.4} {:.4} rg 1 0 0 1 72 {y:.2} Tm {} Tj ET\n",
            f32::from(color.r) / 255.0,
            f32::from(color.g) / 255.0,
            f32::from(color.b) / 255.0,
            s.cid_hex,
        ));
    }

    let stream = doc
        .add_stream(&Buffer::from_bytes(ops.as_bytes()).unwrap(), None, false)
        .unwrap();
    let resources = parse_obj(
        &doc,
        &format!("<< /Font << {} >> >>", font_entries.join(" ")),
    );
    let mut page_obj = doc.load_pdf_page(0).unwrap().object();
    page_obj.dict_put("Resources", resources).unwrap();
    page_obj.dict_put("Contents", stream).unwrap();
    save_mupdf(&doc, true)
}

/// Custom `/Differences` without `/ToUnicode` (maps codes 1,2,3 to non-standard names).
fn sample_custom_encoding_no_tounicode() -> Vec<u8> {
    let content = "BT /F1 18 Tf 72 720 Td <010203> Tj ET\n";
    common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /CustomGlyphFont \
         /FirstChar 1 /LastChar 3 /Widths [600 600 600] \
         /Encoding << /Type /Encoding /Differences [1 /glyph_x1 /glyph_x2 /glyph_x3] >> >>"
            .into(),
    ])
}

/// Vertical writing mode (`wmode = 1`, `/Encoding /UniCNS-UCS2-V`).
fn sample_vertical_cjk() -> Vec<u8> {
    let text = "直書中文測試";
    let hex: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
    let mut bfchars = String::new();
    let mut codes: BTreeSet<u16> = BTreeSet::new();
    for u in text.encode_utf16() {
        codes.insert(u);
    }
    for u in &codes {
        bfchars.push_str(&format!("<{u:04X}> <{u:04X}>\n"));
    }
    let to_unicode = format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
         {} beginbfchar\n{bfchars}endbfchar\n\
         endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
        codes.len()
    );
    let content = format!("BT /F1 20 Tf 300 700 Td <{hex}> Tj ET\n");
    common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type0 /BaseFont /MingLiU /Encoding /UniCNS-UCS2-V \
         /DescendantFonts [6 0 R] /ToUnicode 8 0 R >>"
            .into(),
        "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /MingLiU \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (CNS1) /Supplement 0 >> \
         /FontDescriptor 7 0 R >>"
            .into(),
        "<< /Type /FontDescriptor /FontName /MingLiU /Flags 6 /FontBBox [0 -200 1000 900] \
         /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 880 /StemV 93 >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{to_unicode}\nendstream",
            to_unicode.len()
        ),
    ])
}

/// Complex C-section edge cases:
/// - tight adjacent lines (12pt font, 13.5pt leading)
/// - TJ kerning array
/// - Form XObject text
/// - 30° rotated text with a nearby upright line
/// - skewed (sheared) text
/// - overlapping text (watermark behind text)
/// - link annotation over text
fn sample_edge_cases() -> Vec<u8> {
    let content = "\
BT /F1 12 Tf 0 0 0 rg 72 740 Td (Tight line one above) Tj ET\n\
BT /F1 12 Tf 0 0 0 rg 72 726.5 Td (Target middle line here) Tj ET\n\
BT /F1 12 Tf 0 0 0 rg 72 713 Td (Tight line three below) Tj ET\n\
BT /F1 16 Tf 0 0 0 rg 72 660 Td [(Ker) -30 (ned) -200 (World) -20 (Text)] TJ ET\n\
q /Fm1 Do Q\n\
BT /F1 16 Tf 0 0 0 rg 0.86603 0.5 -0.5 0.86603 72 500 Tm (Rotated thirty degrees) Tj ET\n\
BT /F1 14 Tf 0 0 0 rg 72 568 Td (Above left) Tj ET\n\
BT /F1 16 Tf 0 0 0 rg 1 0 0.3 1 72 420 Tm (Skewed italic shear) Tj ET\n\
BT /F1 18 Tf 0.7 0.7 0.7 rg 72 340 Td (BACKGROUND OVERLAP) Tj ET\n\
BT /F1 18 Tf 0 0 0 rg 72 340 Td (Foreground target) Tj ET\n\
BT /F1 14 Tf 0 0 0.8 rg 72 260 Td (Clickable link line) Tj ET\n";
    let form = "BT /F1 16 Tf 0.1 0.4 0.1 rg 72 610 Td (Inside Form XObject) Tj ET\n";
    common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 6 0 R >> >> \
         /Annots [7 0 R] >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] \
             /Resources << /Font << /F1 5 0 R >> >> /Length {} >>\nstream\n{form}\nendstream",
            form.len()
        ),
        "<< /Type /Annot /Subtype /Link /Rect [70 255 240 278] /Border [0 0 0] \
         /A << /S /URI /URI (https://example.invalid/link) >> >>"
            .into(),
    ])
}

/// Generates a realistic warm cream-paper (`#F5EFE0` with subtle scanner grain) RGB JPEG scanned
/// page with dark charcoal bitmap text (`#242220`) and optionally an invisible `3 Tr` OCR text
/// layer (as ADR 0028 adds).
fn sample_cream_jpeg_scan(with_invisible_ocr_layer: bool) -> (Vec<u8>, Rect) {
    let w = 612usize;
    let h = 792usize;
    let bg = [245i32, 239i32, 224i32]; // warm cream paper #F5EFE0
    let ink = [36u8, 34u8, 28u8]; // dark charcoal
    let mut seed = 0x2545_f491u32;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for _ in 0..(w * h) {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let grain = (seed % 9) as i32 - 4; // subtle ±4 scanner paper grain
        rgb.push((bg[0] + grain).clamp(0, 255) as u8);
        rgb.push((bg[1] + grain).clamp(0, 255) as u8);
        rgb.push((bg[2] + grain).clamp(0, 255) as u8);
    }

    // Draw two lines of crisp block letters ("OLD TITLE" at y=120..148, "KEEP LINE" at y=200..228)
    let glyphs: HashMap<char, [&str; 7]> = HashMap::from([
        (
            ' ',
            [
                "00000", "00000", "00000", "00000", "00000", "00000", "00000",
            ],
        ),
        (
            'O',
            [
                "01110", "10001", "10001", "10001", "10001", "10001", "01110",
            ],
        ),
        (
            'L',
            [
                "10000", "10000", "10000", "10000", "10000", "10000", "11111",
            ],
        ),
        (
            'D',
            [
                "11110", "10001", "10001", "10001", "10001", "10001", "11110",
            ],
        ),
        (
            'T',
            [
                "11111", "00100", "00100", "00100", "00100", "00100", "00100",
            ],
        ),
        (
            'I',
            [
                "01110", "00100", "00100", "00100", "00100", "00100", "01110",
            ],
        ),
        (
            'E',
            [
                "11111", "10000", "10000", "11110", "10000", "10000", "11111",
            ],
        ),
        (
            'K',
            [
                "10001", "10010", "10100", "11000", "10100", "10010", "10001",
            ],
        ),
        (
            'P',
            [
                "11110", "10001", "10001", "11110", "10000", "10000", "10000",
            ],
        ),
        (
            'N',
            [
                "10001", "11001", "10101", "10011", "10001", "10001", "10001",
            ],
        ),
    ]);
    let cell = 4usize;
    let draw_line = |buf: &mut [u8], text: &str, x0: usize, y0: usize| {
        for (i, ch) in text.chars().enumerate() {
            let pat = glyphs[&ch];
            let lx = x0 + i * 6 * cell;
            for (gr, row_str) in pat.iter().enumerate() {
                for (gc, b) in row_str.bytes().enumerate() {
                    if b == b'1' {
                        for dy in 0..cell {
                            for dx in 0..cell {
                                let px = lx + gc * cell + dx;
                                let py = y0 + gr * cell + dy;
                                let at = (py * w + px) * 3;
                                buf[at..at + 3].copy_from_slice(&ink);
                            }
                        }
                    }
                }
            }
        }
    };
    draw_line(&mut rgb, "OLD TITLE", 72, 120);
    draw_line(&mut rgb, "KEEP LINE", 72, 200);

    let mut jpeg = Vec::new();
    let mut enc = Encoder::new(&mut jpeg, 90);
    enc.set_optimized_huffman_tables(true);
    enc.encode(&rgb, w as u16, h as u16, ColorType::Rgb)
        .expect("encode cream scan jpeg");

    let mut doc = MuPdf::from_bytes(&common::one_page("")).expect("blank");
    let img_dict = parse_obj(
        &doc,
        &format!(
            "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode >>"
        ),
    );
    let img_stream = doc
        .add_stream(&Buffer::from_bytes(&jpeg).unwrap(), Some(&img_dict), true)
        .unwrap();
    let font_obj = doc
        .add_object(&parse_obj(
            &doc,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        ))
        .unwrap();

    let mut content = String::from("q 612 0 0 792 0 0 cm /Im1 Do Q\n");
    if with_invisible_ocr_layer {
        // 3 Tr invisible text matching the two lines (PDF y = 792 - page_y)
        content.push_str(
            "BT /F1 24 Tf 3 Tr 72 646 Td (OLD TITLE) Tj ET\n\
             BT /F1 24 Tf 3 Tr 72 566 Td (KEEP LINE) Tj ET\n",
        );
    }
    let cstream = doc
        .add_stream(
            &Buffer::from_bytes(content.as_bytes()).unwrap(),
            None,
            false,
        )
        .unwrap();
    let resources = parse_obj(
        &doc,
        &format!(
            "<< /XObject << /Im1 {} 0 R >> /Font << /F1 {} 0 R >> >>",
            img_stream.as_indirect().unwrap(),
            font_obj.as_indirect().unwrap(),
        ),
    );
    let mut page_obj = doc.load_pdf_page(0).unwrap().object();
    page_obj.dict_put("Resources", resources).unwrap();
    page_obj.dict_put("Contents", cstream).unwrap();

    // OCR bounding box for "OLD TITLE" in page space (72 dpi, y down): x=72..288, y=118..150
    let line1_rect = Rect {
        x0: 70.0,
        y0: 118.0,
        x1: 292.0,
        y1: 150.0,
    };
    (save_mupdf(&doc, true), line1_rect)
}

fn page_plain_text(doc: &PdfDocument, page: u32) -> String {
    doc.page_text(page, 100_000)
        .expect("page_text")
        .lines
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn mean_abs_diff_outside(a: &RenderedPage, b: &RenderedPage, exclude: [usize; 4]) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let [ex0, ey0, ex1, ey1] = exclude;
    let mut sum = 0u64;
    let mut count = 0u64;
    for y in 0..a.height as usize {
        for x in 0..a.width as usize {
            if x >= ex0 && x < ex1 && y >= ey0 && y < ey1 {
                continue;
            }
            let at = (y * a.width as usize + x) * 4;
            for c in 0..3 {
                sum += u64::from(a.rgba[at + c].abs_diff(b.rgba[at + c]));
                count += 1;
            }
        }
    }
    if count == 0 {
        0.0
    } else {
        sum as f64 / count as f64
    }
}

// --------------------------------------------------------------------------- TESTS

/// Question A: Identify font metadata from `TextChar` + page `/Resources /Font`, verify size
/// (including matrix scaling), colour, bold/italic, and measure Windows installed font matching.
#[test]
fn poc_a_identify_fonts_and_match_installed_windows_fonts() {
    let std14 = sample_std14();
    let doc = MuPdf::from_bytes(&std14).expect("open");
    let lines = inspect_page_lines(&doc, 0).expect("inspect");
    assert_eq!(lines.len(), 5);

    // Line 0: Helvetica 18pt Black
    let r0 = lines[0].font_resource.as_ref().expect("font res 0");
    assert_eq!(r0.clean_name, "Helvetica");
    assert_eq!(r0.subtype, "Type1");
    assert!(!r0.embedded);
    assert!(!r0.is_subset);
    assert_eq!(r0.encoding, EncodingKind::WinAnsi);
    assert!(!r0.has_to_unicode);
    assert!((lines[0].size - 18.0).abs() < 0.2, "size={}", lines[0].size);
    assert_eq!(lines[0].color, Rgb { r: 0, g: 0, b: 0 });
    assert!(!lines[0].bold && !lines[0].italic);

    // Line 1: Helvetica-Bold 16pt Red
    let r1 = lines[1].font_resource.as_ref().expect("font res 1");
    assert_eq!(r1.clean_name, "Helvetica-Bold");
    assert!(lines[1].bold);
    assert!((lines[1].size - 16.0).abs() < 0.2);
    assert!(lines[1].color.r >= 200 && lines[1].color.g <= 30 && lines[1].color.b <= 30);

    // Line 2: Times-Italic 14pt Blue
    let r2 = lines[2].font_resource.as_ref().expect("font res 2");
    assert_eq!(r2.clean_name, "Times-Italic");
    assert!(lines[2].italic && lines[2].serif);
    assert!(lines[2].color.b >= 170 && lines[2].color.r <= 30);

    // Line 3: Courier 12pt Green
    assert!(lines[3].monospaced);
    assert!(lines[3].color.g >= 120 && lines[3].color.r <= 60);

    // Line 4: 1 Tf with 24 0 0 24 Tm -> TextChar::size() includes text matrix scaling (~24pt)
    assert!(
        (lines[4].size - 24.0).abs() < 0.2,
        "Tm-scaled size should be ~24pt, got {}",
        lines[4].size
    );

    // Embedded subset check (Type0 Identity-H + ToUnicode + ABCDEF+ prefix)
    let subset_pdf = sample_embedded_subset(
        bundled_droid(),
        0,
        &[(
            "隱私優先 PDF",
            20.0,
            Rgb {
                r: 10,
                g: 20,
                b: 30,
            },
        )],
    );
    let sub_doc = MuPdf::from_bytes(&subset_pdf).expect("open subset");
    let sub_lines = inspect_page_lines(&sub_doc, 0).expect("inspect subset");
    assert_eq!(sub_lines.len(), 1);
    let sr = sub_lines[0].font_resource.as_ref().expect("sub res");
    assert!(sr.is_subset, "raw={}", sr.raw_name);
    assert!(sr.embedded);
    assert_eq!(sr.subtype, "Type0");
    assert_eq!(sr.descendant_subtype.as_deref(), Some("CIDFontType2"));
    assert_eq!(sr.encoding, EncodingKind::IdentityH);
    assert!(sr.has_to_unicode);
    assert!(!sr.unicode_to_code.is_empty());

    // Windows installed fonts matching benchmark
    let catalog = SystemFontCatalog::load_windows_fonts();
    let embeddable_tt = catalog
        .faces
        .iter()
        .filter(|f| f.truetype_embeddable)
        .count();
    let cjk_faces = catalog
        .faces
        .iter()
        .filter(|f| f.truetype_embeddable && f.cjk_capable)
        .count();

    let probe_names = [
        ("Helvetica", false, false),
        ("Helvetica-Bold", true, false),
        ("Helvetica-Oblique", false, true),
        ("Times-Roman", false, false),
        ("Times-BoldItalic", true, true),
        ("Courier", false, false),
        ("ABCDEF+ArialMT", false, false),
        ("ABCDEF+Arial-BoldMT", true, false),
        ("ABCDEF+Arial-ItalicMT", false, true),
        ("GHIJKL+TimesNewRomanPSMT", false, false),
        ("GHIJKL+TimesNewRomanPS-BoldMT", true, false),
        ("MNOPQR+CourierNewPSMT", false, false),
        ("STUVWX+Calibri", false, false),
        ("STUVWX+Calibri-Bold", true, false),
        ("YZABCD+SegoeUI", false, false),
        ("EFGHIJ+Consolas", false, false),
        ("KLMNOP+Georgia", false, false),
        ("QRSTUV+MicrosoftJhengHei", false, false),
        ("QRSTUV+MicrosoftJhengHeiBold", true, false),
        ("WXYZAB+PMingLiU", false, false),
        ("WXYZAB+MingLiU", false, false),
        ("CDEFGH+DFKai-SB", false, false),
        ("IJKLMN+SimSun", false, false),
        ("OPQRST+NonExistentBrandSans-Regular", false, false),
    ];

    let mut matched = 0usize;
    println!(
        "\n=== POC A: Windows Fonts Catalog ({} total faces, {} TrueType embeddable, {} CJK) ===",
        catalog.faces.len(),
        embeddable_tt,
        cjk_faces
    );
    for &(name, b, it) in &probe_names {
        let hit = catalog.match_exact(name, b, it);
        if hit.is_some() {
            matched += 1;
        }
        println!(
            "  {:40} -> {}",
            name,
            hit.map(|f| format!(
                "{} ({}, face={}, bold={}, italic={})",
                f.postscript_name,
                f.path.file_name().unwrap_or_default().to_string_lossy(),
                f.face_index,
                f.bold,
                f.italic
            ))
            .unwrap_or_else(|| "NOT FOUND".to_owned())
        );
    }
    println!(
        "  Matched {matched}/{} ({:.1}%)",
        probe_names.len(),
        matched as f64 * 100.0 / probe_names.len() as f64
    );

    if cfg!(windows) && !catalog.faces.is_empty() {
        // Core Windows fonts (Arial, Times New Roman, Courier New, Segoe UI, Consolas, Georgia)
        // must match on every Windows machine including CI runners.
        assert!(
            catalog
                .match_exact("ABCDEF+ArialMT", false, false)
                .is_some()
        );
        assert!(
            catalog
                .match_exact("ABCDEF+Arial-BoldMT", true, false)
                .is_some()
        );
        assert!(
            catalog
                .match_exact("GHIJKL+TimesNewRomanPSMT", false, false)
                .is_some()
        );
        assert!(catalog.match_exact("Helvetica", false, false).is_some());
        assert!(
            catalog
                .match_exact("OPQRST+NonExistentBrandSans-Regular", false, false)
                .is_none()
        );
        assert!(matched >= 14, "expected at least 14 matches, got {matched}");
    }
}

/// Question B: Can new characters fit in the original PDF font, and comparison of the 3 fallback
/// strategies (exact installed subset vs closest fallback vs refuse) with exact byte sizes.
#[test]
fn poc_b_original_font_glyph_coverage_and_three_fallback_strategies() {
    let catalog = SystemFontCatalog::load_windows_fonts();

    // 1. Build a subset PDF using system Arial (if installed) or Droid Sans Fallback.
    let (arial_bytes, arial_face, is_arial) = catalog
        .match_exact("ArialMT", false, false)
        .and_then(|f| Some((std::fs::read(&f.path).ok()?, f.face_index, true)))
        .unwrap_or_else(|| (bundled_droid().to_vec(), 0, false));

    let subset_pdf = sample_embedded_subset(
        &arial_bytes,
        arial_face,
        &[(
            "Hello World",
            18.0,
            Rgb {
                r: 20,
                g: 40,
                b: 120,
            },
        )],
    );
    let doc = MuPdf::from_bytes(&subset_pdf).expect("open");
    let lines = inspect_page_lines(&doc, 0).expect("inspect");
    let line = &lines[0];
    let res = line.font_resource.as_ref().expect("font res");

    // Characters already in "Hello World" CAN be encoded in the original subset ("Hold"):
    assert!(
        try_encode_in_original_font(res, "Hold").is_ok(),
        "subset already has H, o, l, d"
    );
    // Unseen characters ("Quick Brown Fox") CANNOT fit in the original subset:
    assert!(
        try_encode_in_original_font(res, "Quick Brown Fox").is_err(),
        "subset lacks Q, u, i, c, k, B, n, F, x"
    );

    // Strategy 3: Refuse when original font lacks glyphs
    let refused = edit_line_range(
        &subset_pdf,
        None,
        0,
        line,
        0..line.chars.len(),
        "Quick Brown Fox",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    );
    assert!(refused.is_err());

    // Strategy 1: Same installed font -> embed ADR 0018 subset
    let orig_len = subset_pdf.len();
    let strategy1 = if is_arial {
        let out = edit_line_range(
            &subset_pdf,
            None,
            0,
            line,
            0..line.chars.len(),
            "Quick Brown Fox",
            &catalog,
            FontStrategy::SameFontOrRefuse,
        )
        .expect("strategy 1 exact installed subset");
        assert!(!out.substituted);
        common::check_qpdf_bytes(&out.pdf_bytes);
        let worker = PdfDocument::from_bytes(&out.pdf_bytes).expect("reopen");
        assert!(
            !worker
                .search_page(0, "Quick Brown Fox", true, 5)
                .unwrap()
                .hits
                .is_empty()
        );
        assert!(
            worker
                .search_page(0, "Hello World", true, 5)
                .unwrap()
                .hits
                .is_empty()
        );
        save_poc_pair(
            "b-strategy1-same-installed-arial",
            &subset_pdf,
            &out.pdf_bytes,
            0,
        );
        Some(out)
    } else {
        None
    };

    // Strategy 2: Font NOT installed ("CustomUninstalledSans") -> closest installed fallback
    let uninstalled_pdf = sample_embedded_subset(
        bundled_droid(),
        0,
        &[("Brand Header", 18.0, Rgb { r: 0, g: 0, b: 0 })],
    );
    let udoc = MuPdf::from_bytes(&uninstalled_pdf).expect("open");
    let ulines = inspect_page_lines(&udoc, 0).expect("inspect");
    // SameFontOrRefuse must refuse because DroidSansFallback is not installed in Windows\Fonts
    let s1_missing = edit_line_range(
        &uninstalled_pdf,
        None,
        0,
        &ulines[0],
        0..ulines[0].chars.len(),
        "Quick Update",
        &catalog,
        FontStrategy::SameFontOrRefuse,
    );
    assert!(s1_missing.is_err());
    // BestAvailableFallback succeeds by substituting closest installed font (e.g. Arial)
    let strategy2 = edit_line_range(
        &uninstalled_pdf,
        None,
        0,
        &ulines[0],
        0..ulines[0].chars.len(),
        "Quick Update",
        &catalog,
        FontStrategy::BestAvailableFallback,
    )
    .expect("strategy 2 fallback");
    assert!(strategy2.substituted);
    common::check_qpdf_bytes(&strategy2.pdf_bytes);
    let w2 = PdfDocument::from_bytes(&strategy2.pdf_bytes).expect("reopen s2");
    assert!(
        !w2.search_page(0, "Quick Update", true, 5)
            .unwrap()
            .hits
            .is_empty()
    );
    save_poc_pair(
        "b-strategy2-closest-fallback",
        &uninstalled_pdf,
        &strategy2.pdf_bytes,
        0,
    );

    // CJK subset edit (Traditional Chinese)
    let (cjk_bytes, cjk_face, cjk_label) = catalog
        .match_exact("MicrosoftJhengHei", false, false)
        .and_then(|f| {
            Some((
                std::fs::read(&f.path).ok()?,
                f.face_index,
                "MicrosoftJhengHei",
            ))
        })
        .unwrap_or_else(|| (bundled_droid().to_vec(), 0, "DroidSansFallback"));
    let cjk_pdf = sample_embedded_subset(
        &cjk_bytes,
        cjk_face,
        &[(
            "隱私優先的檔案",
            18.0,
            Rgb {
                r: 10,
                g: 30,
                b: 80,
            },
        )],
    );
    let cdoc = MuPdf::from_bytes(&cjk_pdf).expect("open cjk");
    let clines = inspect_page_lines(&cdoc, 0).expect("inspect cjk");
    let cjk_edited = edit_line_range(
        &cjk_pdf,
        None,
        0,
        &clines[0],
        0..clines[0].chars.len(),
        "修改後的中文內容",
        &catalog,
        FontStrategy::BestAvailableFallback,
    )
    .expect("cjk edit");
    common::check_qpdf_bytes(&cjk_edited.pdf_bytes);
    let wcjk = PdfDocument::from_bytes(&cjk_edited.pdf_bytes).expect("reopen cjk");
    assert!(
        !wcjk
            .search_page(0, "修改後的中文內容", true, 5)
            .unwrap()
            .hits
            .is_empty()
    );
    assert!(
        wcjk.search_page(0, "隱私優先的檔案", true, 5)
            .unwrap()
            .hits
            .is_empty()
    );
    save_poc_pair("b-cjk-type0-subset", &cjk_pdf, &cjk_edited.pdf_bytes, 0);

    println!("\n=== POC B: Fallback Strategies & File Sizes ===");
    if let Some(s1) = strategy1 {
        println!(
            "  Strategy 1 (exact installed Arial): {} -> {} bytes (delta {:+} B, subset={} B, font={})",
            orig_len,
            s1.pdf_bytes.len(),
            s1.pdf_bytes.len() as isize - orig_len as isize,
            s1.subset_bytes,
            s1.font_used
        );
    }
    println!(
        "  Strategy 2 (closest fallback):      {} -> {} bytes (delta {:+} B, subset={} B, font={})",
        uninstalled_pdf.len(),
        strategy2.pdf_bytes.len(),
        strategy2.pdf_bytes.len() as isize - uninstalled_pdf.len() as isize,
        strategy2.subset_bytes,
        strategy2.font_used
    );
    println!(
        "  CJK Type0 Identity-H ({}): {} -> {} bytes (delta {:+} B, subset={} B, font={})",
        cjk_label,
        cjk_pdf.len(),
        cjk_edited.pdf_bytes.len(),
        cjk_edited.pdf_bytes.len() as isize - cjk_pdf.len() as isize,
        cjk_edited.subset_bytes,
        cjk_edited.font_used
    );
}

/// Question C: Edit one line — per-char quad removal, style/position/rotation/skew preservation,
/// edge cases (adjacent lines, same-line other words, shorter/longer gap & overlap, TJ arrays,
/// Form XObject, rotated pages, overlapping text, link annotation collateral deletion), qpdf,
/// save/reopen, and ADR 0013 undo replay determinism.
#[test]
fn poc_c_edit_single_line_removal_placement_edge_cases_and_undo() {
    let catalog = SystemFontCatalog::load_windows_fonts();

    // 1. Standard 14 edit: preserve font, size, colour; verify search, copy, qpdf, save/reopen, undo replay
    let std14 = sample_std14();
    let doc = MuPdf::from_bytes(&std14).expect("open");
    let lines = inspect_page_lines(&doc, 0).expect("inspect");
    let edited = edit_line_range(
        &std14,
        None,
        0,
        &lines[1], // Helvetica Bold Red Line
        0..lines[1].chars.len(),
        "Updated Bold Red Title",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    )
    .expect("edit std14 line 1");
    common::check_qpdf_bytes(&edited.pdf_bytes);
    save_poc_pair("c-std14-bold-red", &std14, &edited.pdf_bytes, 0);

    let worker = PdfDocument::from_bytes(&edited.pdf_bytes).expect("worker open");
    assert!(
        !worker
            .search_page(0, "Updated Bold Red Title", true, 5)
            .unwrap()
            .hits
            .is_empty()
    );
    assert!(
        worker
            .search_page(0, "Helvetica Bold Red Line", true, 5)
            .unwrap()
            .hits
            .is_empty()
    );
    let copied = page_plain_text(&worker, 0);
    assert!(copied.contains("Updated Bold Red Title"));
    assert!(copied.contains("Helvetica 18pt Black Line"));
    assert!(copied.contains("Times Italic Blue Line"));

    // Save through worker's own `save()` and reopen
    let mut resaved = Vec::new();
    worker.save(&mut resaved).expect("worker save");
    common::check_qpdf_bytes(&resaved);
    let reopened = PdfDocument::from_bytes(&resaved).expect("reopen");
    assert_eq!(page_plain_text(&reopened, 0), copied);
    assert_eq!(
        worker.render(0, 1.0, 0).unwrap().rgba,
        reopened.render(0, 1.0, 0).unwrap().rgba
    );

    // Verify edited line kept red colour and ~16pt size
    let re_mu = MuPdf::from_bytes(&resaved).expect("re_mu");
    let re_lines = inspect_page_lines(&re_mu, 0).expect("re_lines");
    let updated_line = re_lines
        .iter()
        .find(|l| l.text.contains("Updated Bold Red Title"))
        .expect("updated line");
    assert!((updated_line.size - 16.0).abs() < 0.2);
    assert!(updated_line.color.r >= 200 && updated_line.color.g <= 30);

    // ADR 0013 Undo / deterministic replay from original bytes
    let replay = edit_line_range(
        &std14,
        None,
        0,
        &lines[1],
        0..lines[1].chars.len(),
        "Updated Bold Red Title",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    )
    .expect("replay");
    let replay_worker = PdfDocument::from_bytes(&replay.pdf_bytes).expect("replay worker");
    assert_eq!(
        worker.render(0, 1.0, 0).unwrap().rgba,
        replay_worker.render(0, 1.0, 0).unwrap().rgba,
        "replaying the same edit from original bytes produces identical pixels"
    );
    assert_eq!(
        page_plain_text(&worker, 0),
        page_plain_text(&replay_worker, 0)
    );

    // 2. Partial-line edits (shorter -> gap, longer -> overlap) without moving trailing words
    let first_word_len = "Helvetica".len();
    let old_word_width = lines[0].chars[first_word_len - 1].quad.ur.x - lines[0].chars[0].quad.ul.x;
    let shorter = edit_line_range(
        &std14,
        None,
        0,
        &lines[0],
        0..first_word_len,
        "Hi",
        &catalog,
        FontStrategy::BestAvailableFallback,
    )
    .expect("shorter");
    let longer = edit_line_range(
        &std14,
        None,
        0,
        &lines[0],
        0..first_word_len,
        "ExtraordinarilyLongPrefix",
        &catalog,
        FontStrategy::BestAvailableFallback,
    )
    .expect("longer");
    common::check_qpdf_bytes(&shorter.pdf_bytes);
    common::check_qpdf_bytes(&longer.pdf_bytes);
    save_poc_pair("c-shorter-gap", &std14, &shorter.pdf_bytes, 0);
    save_poc_pair("c-longer-overlap", &std14, &longer.pdf_bytes, 0);

    let w_short = PdfDocument::from_bytes(&shorter.pdf_bytes).unwrap();
    assert!(
        !w_short
            .search_page(0, "18pt Black Line", true, 5)
            .unwrap()
            .hits
            .is_empty(),
        "trailing words on the same line stay intact"
    );
    let w_long = PdfDocument::from_bytes(&longer.pdf_bytes).unwrap();
    assert!(
        !w_long
            .search_page(0, "18pt Black Line", true, 5)
            .unwrap()
            .hits
            .is_empty()
    );
    let gap_pt = old_word_width - shorter.new_width_pt;
    let overlap_pt = longer.new_width_pt - old_word_width;

    // 3. Complex edge cases sample
    let edge = sample_edge_cases();
    let edoc = MuPdf::from_bytes(&edge).expect("open edge");
    let elines = inspect_page_lines(&edoc, 0).expect("inspect edge");

    // 3a. Tight adjacent lines (12pt font, 13.5pt leading):
    // First, control with raw (non-inset) char quads: because TextChar::quad spans full
    // ascender..descender (~1.16 * size), it reaches into line 1's inner 80% and deletes it!
    let mid_line = elines
        .iter()
        .find(|l| l.text.contains("Target middle line"))
        .expect("mid line");
    let raw_quads_damaged_upper = {
        let mdoc = MuPdf::from_bytes(&edge).unwrap();
        {
            let mut p = mdoc.load_pdf_page(0).unwrap();
            let raw_qs: Vec<Quad> = mid_line
                .chars
                .iter()
                .filter(|c| !c.ch.is_whitespace())
                .map(|c| c.quad.clone())
                .collect();
            p.add_redact_annotation(raw_qs).unwrap();
            p.redact_with_options(PdfRedactOptions {
                black_boxes: false,
                image_method: PdfRedactImageMethod::None,
                line_art: PdfRedactLineArtMethod::None,
                text: PdfRedactTextMethod::Remove,
            })
            .unwrap();
        }
        let w = PdfDocument::from_bytes(&save_mupdf(&mdoc, true)).unwrap();
        !page_plain_text(&w, 0).contains("Tight line one above")
    };
    // With 15% vertical inset on per-char quads, both tight adjacent lines are spared!
    let mid_out = edit_line_range(
        &edge,
        None,
        0,
        mid_line,
        0..mid_line.chars.len(),
        "Edited middle line",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    )
    .expect("edit mid");
    let w_mid = PdfDocument::from_bytes(&mid_out.pdf_bytes).unwrap();
    let mid_txt = page_plain_text(&w_mid, 0);
    assert!(
        mid_txt.contains("Tight line one above"),
        "upper line kept with vertically-inset quads: {mid_txt}"
    );
    assert!(
        mid_txt.contains("Tight line three below"),
        "lower line kept with vertically-inset quads: {mid_txt}"
    );
    assert!(mid_txt.contains("Edited middle line"));
    assert!(!mid_txt.contains("Target middle line"));

    // 3b. TJ array partial edit ("Kerned" -> "New", leaving "WorldText" untouched)
    let tj_line = elines
        .iter()
        .find(|l| l.text.contains("Kerned"))
        .expect("tj line");
    let tj_out = edit_line_range(
        &edge,
        None,
        0,
        tj_line,
        0.."Kerned".len(),
        "Edited",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    )
    .expect("edit tj");
    let w_tj = PdfDocument::from_bytes(&tj_out.pdf_bytes).unwrap();
    let tj_txt = page_plain_text(&w_tj, 0);
    assert!(tj_txt.contains("Edited"), "new TJ word present");
    assert!(
        tj_txt.contains("WorldText"),
        "rest of TJ array preserved: {tj_txt}"
    );
    assert!(!tj_txt.contains("Kerned"), "old TJ word removed");

    // 3c. Form XObject text edit
    let fm_line = elines
        .iter()
        .find(|l| l.text.contains("Inside Form XObject"))
        .expect("form line");
    let fm_out = edit_line_range(
        &edge,
        None,
        0,
        fm_line,
        0..fm_line.chars.len(),
        "Edited Form Text",
        &catalog,
        FontStrategy::BestAvailableFallback,
    )
    .expect("edit form");
    common::check_qpdf_bytes(&fm_out.pdf_bytes);
    let w_fm = PdfDocument::from_bytes(&fm_out.pdf_bytes).unwrap();
    let fm_txt = page_plain_text(&w_fm, 0);
    assert!(fm_txt.contains("Edited Form Text"));
    assert!(!fm_txt.contains("Inside Form XObject"));

    // 3d. 30° rotated text: per-char quads spare nearby upright line, whereas whole-line bbox deletes it!
    let rot_line = elines
        .iter()
        .find(|l| l.text.contains("Rotated thirty"))
        .expect("rot line");
    let rot_out = edit_line_range(
        &edge,
        None,
        0,
        rot_line,
        0..rot_line.chars.len(),
        "Slanted replacement",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    )
    .expect("edit rotated text");
    common::check_qpdf_bytes(&rot_out.pdf_bytes);
    save_poc_pair("c-rotated-30deg", &edge, &rot_out.pdf_bytes, 0);
    let w_rot = PdfDocument::from_bytes(&rot_out.pdf_bytes).unwrap();
    let rot_txt = page_plain_text(&w_rot, 0);
    assert!(
        rot_txt.contains("Above left"),
        "per-char quads spare nearby upright line: {rot_txt}"
    );
    assert!(rot_txt.contains("Slanted replacement"));
    assert!(!rot_txt.contains("Rotated thirty"));

    // Control: whole-line bbox redaction on the 30° rotated text swallows the nearby upright line!
    let bbox_swallowed = {
        let mdoc = MuPdf::from_bytes(&edge).unwrap();
        {
            let mut p = mdoc.load_pdf_page(0).unwrap();
            p.add_redact_annotation(rot_line.bounds).unwrap();
            p.redact_with_options(PdfRedactOptions {
                black_boxes: false,
                image_method: PdfRedactImageMethod::None,
                line_art: PdfRedactLineArtMethod::None,
                text: PdfRedactTextMethod::Remove,
            })
            .unwrap();
        }
        let w = PdfDocument::from_bytes(&save_mupdf(&mdoc, true)).unwrap();
        !page_plain_text(&w, 0).contains("Above left")
    };
    assert!(
        bbox_swallowed,
        "control: whole-line bbox accidentally deletes nearby upright line"
    );

    // 3e. Skewed (sheared) text
    let skew_line = elines
        .iter()
        .find(|l| l.text.contains("Skewed italic shear"))
        .expect("skew line");
    let skew_out = edit_line_range(
        &edge,
        None,
        0,
        skew_line,
        0..skew_line.chars.len(),
        "Updated shear text",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    )
    .expect("edit skewed");
    common::check_qpdf_bytes(&skew_out.pdf_bytes);
    let w_skew = PdfDocument::from_bytes(&skew_out.pdf_bytes).unwrap();
    assert!(page_plain_text(&w_skew, 0).contains("Updated shear text"));

    // 3f. Overlapping text: prove `pdf_redact_text_filter` also removes overlapping background text!
    let fg_line = elines
        .iter()
        .find(|l| l.text.contains("Foreground target"))
        .expect("fg line");
    let ov_out = edit_line_range(
        &edge,
        None,
        0,
        fg_line,
        0..fg_line.chars.len(),
        "New foreground",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    )
    .expect("edit overlap");
    let w_ov = PdfDocument::from_bytes(&ov_out.pdf_bytes).unwrap();
    let bg_damaged = !page_plain_text(&w_ov, 0).contains("BACKGROUND OVERLAP");
    assert!(
        bg_damaged,
        "finding: geometry-only pdf_redact_text_filter removes overlapping background text too"
    );

    // 3g. Link annotation over text: prove `pdf_redact_page_links` deletes links in the area!
    let link_line = elines
        .iter()
        .find(|l| l.text.contains("Clickable link line"))
        .expect("link line");
    let links_before = PdfDocument::from_bytes(&edge)
        .unwrap()
        .page_links(0, 10)
        .unwrap()
        .len();
    assert_eq!(links_before, 1);
    let link_out = edit_line_range(
        &edge,
        None,
        0,
        link_line,
        0..link_line.chars.len(),
        "Edited link text",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    )
    .expect("edit link line");
    let links_after = PdfDocument::from_bytes(&link_out.pdf_bytes)
        .unwrap()
        .page_links(0, 10)
        .unwrap()
        .len();
    assert_eq!(
        links_after, 0,
        "finding: pdf_redact_page unconditionally calls pdf_redact_page_links and deletes overlapping Link annotations"
    );

    // 4. Rotated page (`/Rotate 90` in corpus `benign/rotated-page.pdf`)
    let rot_page_pdf = corpus_bytes("benign/rotated-page.pdf");
    let rdoc = MuPdf::from_bytes(&rot_page_pdf).unwrap();
    let rlines = inspect_page_lines(&rdoc, 1).unwrap();
    assert_eq!(rlines.len(), 1);
    let rp_out = edit_line_range(
        &rot_page_pdf,
        None,
        1,
        &rlines[0],
        0..rlines[0].chars.len(),
        "Page 2: Edited Rotated",
        &catalog,
        FontStrategy::OriginalOnlyOrRefuse,
    )
    .expect("edit rotated page");
    common::check_qpdf_bytes(&rp_out.pdf_bytes);
    save_poc_pair("c-rotated-page-90", &rot_page_pdf, &rp_out.pdf_bytes, 1);
    let w_rp = PdfDocument::from_bytes(&rp_out.pdf_bytes).unwrap();
    let rp_txt = page_plain_text(&w_rp, 1);
    assert!(rp_txt.contains("Page 2: Edited Rotated"));
    assert!(!rp_txt.contains("/Rotate 90"));

    println!("\n=== POC C: Single-Line Edit Edge Cases ===");
    println!("  Shorter replacement gap:    {gap_pt:.1} pt");
    println!("  Longer replacement overlap: {overlap_pt:.1} pt");
    println!(
        "  Tight 13.5pt leading (12pt font): raw quads damaged upper={raw_quads_damaged_upper}; 15% vertical-inset quads spared both!"
    );
    println!(
        "  30deg rotated text: per-char quads spared nearby line; whole-line bbox swallowed={bbox_swallowed}"
    );
    println!("  Overlapping background text damaged by pdf_redact_text_filter: {bg_damaged}");
    println!("  Link annotation deleted by pdf_redact_page_links: {links_before} -> {links_after}");
}

/// Question D: Scanned pages (Option 1) — compare MuPDF `Pixels` + `black_boxes: false` vs
/// worker-side custom background-colour fill + JPEG recompression + `3 Tr` removal + new text.
#[test]
fn poc_d_scanned_page_option1_mupdf_pixels_vs_custom_background_fill() {
    // 1. Cream-paper JPEG scan (`#F5EFE0` paper, `#242220` ink) WITH pre-existing 3 Tr OCR layer
    let (cream_scan, line1_rect) = sample_cream_jpeg_scan(true);
    let before_worker = PdfDocument::from_bytes(&cream_scan).expect("open cream scan");
    assert!(
        !before_worker
            .search_page(0, "OLD TITLE", true, 5)
            .unwrap()
            .hits
            .is_empty(),
        "pre-existing 3 Tr invisible text is searchable before edit"
    );

    // 1a. Test MuPDF's built-in `Pixels` + `black_boxes: false`
    let mupdf_redacted = {
        let doc = MuPdf::from_bytes(&cream_scan).unwrap();
        {
            let mut page = doc.load_pdf_page(0).unwrap();
            page.add_redact_annotation(line1_rect).unwrap();
            page.redact_with_options(PdfRedactOptions {
                black_boxes: false,
                image_method: PdfRedactImageMethod::Pixels,
                line_art: PdfRedactLineArtMethod::None,
                text: PdfRedactTextMethod::Remove,
            })
            .unwrap();
        }
        save_mupdf(&doc, true)
    };
    let mupdf_rendered = PdfDocument::from_bytes(&mupdf_redacted)
        .unwrap()
        .render(0, 1.0, 0)
        .unwrap();
    // Sample a pixel in the middle of the redacted box (x=150, y=134):
    let at = (134 * mupdf_rendered.width as usize + 150) * 4;
    let mupdf_fill = [
        mupdf_rendered.rgba[at],
        mupdf_rendered.rgba[at + 1],
        mupdf_rendered.rgba[at + 2],
    ];
    assert_eq!(
        mupdf_fill,
        [255, 255, 255],
        "MuPDF pdf_redact_image_imp always fills with pure white (255), leaving a visible patch on cream paper #F5EFE0"
    );
    save_poc_pair("d-mupdf-white-patch", &cream_scan, &mupdf_redacted, 0);

    // 1b. Worker-side custom background fill + JPEG recompression + 3 Tr removal + new visible text
    let (custom_edited, bg, ink) =
        edit_scanned_page_option1(&cream_scan, line1_rect, "NEW TITLE 2026", 90)
            .expect("custom scan edit");
    common::check_qpdf_bytes(&custom_edited);
    save_poc_pair("d-cream-scan-option1", &cream_scan, &custom_edited, 0);

    // Background colour sampled from surrounding cream paper should match ~[245, 239, 224]
    assert!(
        bg[0].abs_diff(245) <= 4 && bg[1].abs_diff(239) <= 4 && bg[2].abs_diff(224) <= 4,
        "sampled paper bg={bg:?}"
    );
    // Ink colour sampled from charcoal text should match ~[36, 34, 32]
    assert!(
        ink[0] < 80 && ink[1] < 80 && ink[2] < 80,
        "sampled ink={ink:?}"
    );

    let after_worker = PdfDocument::from_bytes(&custom_edited).unwrap();
    // Old 3 Tr text is gone; new visible text is searchable & copyable; second line's 3 Tr text stays!
    assert!(
        after_worker
            .search_page(0, "OLD TITLE", true, 5)
            .unwrap()
            .hits
            .is_empty(),
        "old 3 Tr text removed"
    );
    assert!(
        !after_worker
            .search_page(0, "NEW TITLE 2026", true, 5)
            .unwrap()
            .hits
            .is_empty(),
        "new text searchable"
    );
    assert!(
        !after_worker
            .search_page(0, "KEEP LINE", true, 5)
            .unwrap()
            .hits
            .is_empty(),
        "unmodified line still present"
    );

    // Check pixel fidelity outside the edited box after JPEG recompression
    let before_render = before_worker.render(0, 1.0, 0).unwrap();
    let after_render = after_worker.render(0, 1.0, 0).unwrap();
    let mad_outside = mean_abs_diff_outside(&before_render, &after_render, [65, 110, 340, 160]);
    assert!(
        mad_outside < 2.0,
        "JPEG recompression outside edited area should stay crisp: MAD={mad_outside:.3}"
    );

    // Verify background right edge inside cleared box (where no new glyph is drawn, e.g. x=285, y=134)
    let at_bg = (134 * after_render.width as usize + 285) * 4;
    let cleared_px = [
        after_render.rgba[at_bg],
        after_render.rgba[at_bg + 1],
        after_render.rgba[at_bg + 2],
    ];
    assert!(
        cleared_px[0].abs_diff(245) <= 6
            && cleared_px[1].abs_diff(239) <= 6
            && cleared_px[2].abs_diff(224) <= 6,
        "cleared scan region matches cream paper: {cleared_px:?}"
    );

    // 2. Corpus 1-bit RunLengthDecode scan (`tests/corpus/benign/scanned-text.pdf`)
    // Line 1 ("PRIVACY FIRST") is at image pixels x=100..646, y=300..349 on 1224x1584 ->
    // page space (612x792): x=50..323, y=150..175.
    let corpus_scan = corpus_bytes("benign/scanned-text.pdf");
    let scan_line_rect = Rect {
        x0: 48.0,
        y0: 148.0,
        x1: 330.0,
        y1: 178.0,
    };
    let (corpus_scan_edited, c_bg, _c_ink) =
        edit_scanned_page_option1(&corpus_scan, scan_line_rect, "EDITED SCAN LINE", 90)
            .expect("corpus scanned-text.pdf edit");
    common::check_qpdf_bytes(&corpus_scan_edited);
    save_poc_pair(
        "d-corpus-scanned-text",
        &corpus_scan,
        &corpus_scan_edited,
        0,
    );
    let w_cscan = PdfDocument::from_bytes(&corpus_scan_edited).unwrap();
    assert!(
        !w_cscan
            .search_page(0, "EDITED SCAN LINE", true, 5)
            .unwrap()
            .hits
            .is_empty()
    );
    assert_eq!(c_bg, [255, 255, 255]);

    println!("\n=== POC D: Scanned Page (Option 1) ===");
    println!("  Cream JPEG scan original: {} B", cream_scan.len());
    println!(
        "  MuPDF Pixels (black_boxes=false): {} B, fill={mupdf_fill:?} (always pure white, Flate bloat)",
        mupdf_redacted.len()
    );
    println!(
        "  Worker custom JPEG Q90 fill: {} B (delta {:+} B), sampled bg={bg:?}, ink={ink:?}, outside MAD={mad_outside:.3}",
        custom_edited.len(),
        custom_edited.len() as isize - cream_scan.len() as isize,
    );
    println!(
        "  Corpus 1-bit scanned-text.pdf: {} B -> {} B",
        corpus_scan.len(),
        corpus_scan_edited.len()
    );
}

/// Question E: Detect and refuse all 6 unsupported / disallowed cases cleanly.
#[test]
fn poc_e_uneditable_and_refused_cases_detection() {
    // 1. Type 3 font (`tests/corpus/benign/type3-decimal-glyph-names.pdf`)
    // Line 0 is the Helvetica title ("Type 3 glyph names"); line 1 is the Type 3 text ("Hello World 2026").
    let type3_bytes = corpus_bytes("benign/type3-decimal-glyph-names.pdf");
    let w_t3 = PdfDocument::from_bytes(&type3_bytes).unwrap();
    let m_t3 = MuPdf::from_bytes(&type3_bytes).unwrap();
    let l_t3 = inspect_page_lines(&m_t3, 0).unwrap();
    let t3_line = l_t3
        .iter()
        .find(|l| l.text.contains("Hello World 2026"))
        .expect("Type 3 line");
    assert_eq!(
        t3_line.font_resource.as_ref().map(|r| r.subtype.as_str()),
        Some("Type3")
    );
    assert_eq!(
        check_line_editability(&w_t3, t3_line),
        Err(RefusalReason::Type3Font)
    );
    let t3_title = l_t3
        .iter()
        .find(|l| l.text.contains("Type 3 glyph names"))
        .expect("Helvetica title");
    assert_eq!(check_line_editability(&w_t3, t3_title), Ok(()));

    // 2. Custom encoding without /ToUnicode
    let custom_bytes = sample_custom_encoding_no_tounicode();
    let w_ce = PdfDocument::from_bytes(&custom_bytes).unwrap();
    let m_ce = MuPdf::from_bytes(&custom_bytes).unwrap();
    let l_ce = inspect_page_lines(&m_ce, 0).unwrap();
    assert!(!l_ce.is_empty());
    assert_eq!(
        check_line_editability(&w_ce, &l_ce[0]),
        Err(RefusalReason::CustomEncodingNoToUnicode)
    );

    // 3. Vertical writing (`wmode == 1`)
    let vert_bytes = sample_vertical_cjk();
    let w_v = PdfDocument::from_bytes(&vert_bytes).unwrap();
    let m_v = MuPdf::from_bytes(&vert_bytes).unwrap();
    let l_v = inspect_page_lines(&m_v, 0).unwrap();
    assert!(!l_v.is_empty());
    assert_eq!(l_v[0].wmode, WriteMode::Vertical);
    assert_eq!(
        check_line_editability(&w_v, &l_v[0]),
        Err(RefusalReason::VerticalWriting)
    );

    // 4. Clip-only text (`7 Tr`) in `tests/corpus/benign/text-render-modes.pdf` and invisible
    //    `3 Tr` text in a scanned OCR PDF
    let tr_bytes = corpus_bytes("benign/text-render-modes.pdf");
    let w_tr = PdfDocument::from_bytes(&tr_bytes).unwrap();
    let m_tr = MuPdf::from_bytes(&tr_bytes).unwrap();
    let l_tr = inspect_page_lines(&m_tr, 0).unwrap();
    let mode7 = l_tr
        .iter()
        .find(|l| l.text.contains("Mode7ClipOnly"))
        .expect("Mode7ClipOnly line");
    assert_eq!(
        check_line_editability(&w_tr, mode7),
        Err(RefusalReason::InvisibleOrClipOnlyText)
    );
    let mixed = l_tr
        .iter()
        .find(|l| l.text.contains("MixedClip"))
        .expect("MixedClip line");
    assert_eq!(
        check_line_editability(&w_tr, mixed),
        Err(RefusalReason::InvisibleOrClipOnlyText)
    );
    let mode0 = l_tr
        .iter()
        .find(|l| l.text.contains("Mode0Fill"))
        .expect("Mode0Fill line");
    assert_eq!(check_line_editability(&w_tr, mode0), Ok(()));
    let (ocr_scan, _) = sample_cream_jpeg_scan(true);
    let w_ocr = PdfDocument::from_bytes(&ocr_scan).unwrap();
    let m_ocr = MuPdf::from_bytes(&ocr_scan).unwrap();
    let l_ocr = inspect_page_lines(&m_ocr, 0).unwrap();
    assert_eq!(
        check_line_editability(&w_ocr, &l_ocr[0]),
        Err(RefusalReason::InvisibleOrClipOnlyText)
    );

    // 5. Signed document (`tests/corpus/benign/signed.pdf`)
    let signed_bytes = corpus_bytes("benign/signed.pdf");
    let w_sig = PdfDocument::from_bytes(&signed_bytes).unwrap();
    let m_sig = MuPdf::from_bytes(&signed_bytes).unwrap();
    let l_sig = inspect_page_lines(&m_sig, 0).unwrap();
    assert_eq!(
        check_line_editability(&w_sig, &l_sig[0]),
        Err(RefusalReason::SignedDocument)
    );

    // 6. Author forbids modification `/P` (`tests/corpus/benign/restricted-no-modify.pdf`)
    let rest_bytes = corpus_bytes("benign/restricted-no-modify.pdf");
    let w_rest = PdfDocument::from_bytes(&rest_bytes).unwrap();
    let m_rest = MuPdf::from_bytes(&rest_bytes).unwrap();
    let l_rest = inspect_page_lines(&m_rest, 0).unwrap();
    assert_eq!(
        check_line_editability(&w_rest, &l_rest[0]),
        Err(RefusalReason::ModifyForbidden)
    );
}

/// Question F: Sweep every PDF in `tests/corpus` (benign, malicious, malformed) plus generated
/// synthetic samples (Std14, TrueType subset, Type0 Identity-H CJK, Type3, custom encoding,
/// vertical) and measure A (font identification + Windows match), B (original font capacity),
/// and C (first-word edit success / refusal / failure breakdown).
#[test]
fn poc_f_corpus_and_generated_samples_measurement() {
    let catalog = SystemFontCatalog::load_windows_fonts();

    // Collect every PDF in tests/corpus/{benign,malicious,malformed}
    let mut corpus_files: Vec<(String, Vec<u8>, Option<&'static str>)> = Vec::new();
    for category in ["benign", "malicious", "malformed"] {
        let dir = corpus_root().join(category);
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("read corpus category")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("pdf"))
            .collect();
        entries.sort();
        for path in entries {
            let rel = format!("{category}/{}", path.file_name().unwrap().to_string_lossy());
            let bytes = std::fs::read(&path).unwrap();
            let pw = if rel.contains("encrypted-") || rel.contains("restricted-open-password") {
                Some("user")
            } else {
                None
            };
            corpus_files.push((rel, bytes, pw));
        }
    }

    // Add generated synthetic samples (saved to poc-593/ too)
    let (arial_bytes, arial_face) = catalog
        .match_exact("ArialMT", false, false)
        .and_then(|f| Some((std::fs::read(&f.path).ok()?, f.face_index)))
        .unwrap_or_else(|| (bundled_droid().to_vec(), 0));
    let (cjk_bytes, cjk_face) = catalog
        .match_exact("MicrosoftJhengHei", false, false)
        .and_then(|f| Some((std::fs::read(&f.path).ok()?, f.face_index)))
        .unwrap_or_else(|| (bundled_droid().to_vec(), 0));

    let synthetic_samples: Vec<(String, Vec<u8>, Option<&'static str>)> = vec![
        ("synthetic/std14.pdf".to_owned(), sample_std14(), None),
        (
            "synthetic/truetype-subset-en.pdf".to_owned(),
            sample_embedded_subset(
                &arial_bytes,
                arial_face,
                &[
                    ("Quarterly Report 2026", 18.0, Rgb { r: 0, g: 0, b: 0 }),
                    (
                        "Confidential Draft",
                        14.0,
                        Rgb {
                            r: 160,
                            g: 20,
                            b: 20,
                        },
                    ),
                ],
            ),
            None,
        ),
        (
            "synthetic/type0-identity-h-cjk.pdf".to_owned(),
            sample_embedded_subset(
                &cjk_bytes,
                cjk_face,
                &[
                    ("機密合約書範本", 18.0, Rgb { r: 0, g: 0, b: 0 }),
                    (
                        "甲方與乙方同意以下條款",
                        12.0,
                        Rgb {
                            r: 40,
                            g: 40,
                            b: 40,
                        },
                    ),
                ],
            ),
            None,
        ),
        (
            "synthetic/custom-enc-no-tounicode.pdf".to_owned(),
            sample_custom_encoding_no_tounicode(),
            None,
        ),
        (
            "synthetic/vertical-cjk.pdf".to_owned(),
            sample_vertical_cjk(),
            None,
        ),
        (
            "synthetic/edge-cases.pdf".to_owned(),
            sample_edge_cases(),
            None,
        ),
    ];

    for (name, bytes, _) in &synthetic_samples {
        let fname = name.replace('/', "-");
        save_poc_artifact(&fname, bytes);
    }

    #[derive(Default)]
    struct Stats {
        docs_total: usize,
        docs_opened: usize,
        docs_unopenable: usize,
        pages_total: usize,
        pages_with_text: usize,
        pages_no_text: usize,
        lines_total: usize,
        a_font_dict_matched: usize,
        a_windows_exact_matched: usize,
        a_subtypes: BTreeMap<String, usize>,
        a_encodings: BTreeMap<String, usize>,
        b_fits_original_font: usize,
        b_needs_new_subset: usize,
        c_edit_ok: usize,
        c_edit_ok_verified: usize,
        c_refused: BTreeMap<String, usize>,
        c_failed: BTreeMap<String, usize>,
    }

    let measure_set = |label: &str, items: &[(String, Vec<u8>, Option<&'static str>)]| -> Stats {
        let mut st = Stats {
            docs_total: items.len(),
            ..Stats::default()
        };
        for (name, bytes, pw) in items {
            let Ok(worker_doc) = PdfDocument::open(bytes, *pw) else {
                st.docs_unopenable += 1;
                continue;
            };
            let Ok(mut mupdf_doc) = MuPdf::from_bytes(bytes) else {
                st.docs_unopenable += 1;
                continue;
            };
            if mupdf_doc.needs_password().unwrap_or(false)
                && pw.is_none_or(|p| mupdf_doc.authenticate(p).is_err())
            {
                st.docs_unopenable += 1;
                continue;
            }
            let Ok(page_count) = worker_doc.page_count() else {
                st.docs_unopenable += 1;
                continue;
            };
            if page_count == 0 {
                st.docs_unopenable += 1;
                continue;
            }
            st.docs_opened += 1;

            for page_idx in 0..page_count {
                st.pages_total += 1;
                let Ok(lines) = inspect_page_lines(&mupdf_doc, page_idx as i32) else {
                    st.pages_no_text += 1;
                    continue;
                };
                if lines.is_empty() {
                    st.pages_no_text += 1;
                    continue;
                }
                st.pages_with_text += 1;

                for line in &lines {
                    st.lines_total += 1;

                    // A: font identification + Windows match
                    let pdf_font_name = if let Some(res) = &line.font_resource {
                        st.a_font_dict_matched += 1;
                        *st.a_subtypes.entry(res.subtype.clone()).or_insert(0) += 1;
                        *st.a_encodings.entry(res.encoding.label()).or_insert(0) += 1;
                        res.clean_name.as_str()
                    } else {
                        *st.a_subtypes.entry("Unmatched".to_owned()).or_insert(0) += 1;
                        println!(
                            "  [UNMATCHED] {name} p{page_idx}: text={:?} fz_font={:?}",
                            line.text, line.chars[0].font_name
                        );
                        line.chars[0].font_name.as_str()
                    };
                    if catalog
                        .match_exact(pdf_font_name, line.bold, line.italic)
                        .is_some()
                    {
                        st.a_windows_exact_matched += 1;
                    }

                    // Pick first word range (non-whitespace prefix)
                    let start = line
                        .chars
                        .iter()
                        .position(|c| !c.ch.is_whitespace())
                        .unwrap_or(0);
                    let end = line.chars[start..]
                        .iter()
                        .position(|c| c.ch.is_whitespace())
                        .map_or(line.chars.len(), |off| start + off);
                    let first_word: String = line.chars[start..end].iter().map(|c| c.ch).collect();
                    let is_cjk_line = first_word.chars().any(|c| u32::from(c) > 0x2E7F);
                    let replacement = if is_cjk_line {
                        "修改詞"
                    } else {
                        "EditedWord"
                    };

                    // B: can replacement fit in original font alone?
                    if let Some(res) = &line.font_resource
                        && try_encode_in_original_font(res, replacement).is_ok()
                    {
                        st.b_fits_original_font += 1;
                    } else {
                        st.b_needs_new_subset += 1;
                    }

                    // C: attempt edit of first word
                    match check_line_editability(&worker_doc, line) {
                        Err(reason) => {
                            *st.c_refused.entry(format!("{reason:?}")).or_insert(0) += 1;
                        }
                        Ok(()) => {
                            match edit_line_range(
                                bytes,
                                *pw,
                                page_idx,
                                line,
                                start..end,
                                replacement,
                                &catalog,
                                FontStrategy::BestAvailableFallback,
                            ) {
                                Ok(out) => {
                                    st.c_edit_ok += 1;
                                    if let Ok(reopen) = PdfDocument::open(&out.pdf_bytes, *pw)
                                        && let Ok(found) =
                                            reopen.search_page(page_idx, replacement, true, 5)
                                        && !found.hits.is_empty()
                                    {
                                        st.c_edit_ok_verified += 1;
                                    } else {
                                        *st.c_failed
                                            .entry(format!("search-miss:{name}:p{page_idx}"))
                                            .or_insert(0) += 1;
                                    }
                                }
                                Err(err) => {
                                    println!(
                                        "  [EDIT-FAIL] {name} p{page_idx}: text={:?} err={err}",
                                        line.text
                                    );
                                    *st.c_failed.entry(err).or_insert(0) += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        println!("\n=== POC F: {label} ===");
        println!(
            "  Docs: {} total, {} opened, {} unopenable/0-page",
            st.docs_total, st.docs_opened, st.docs_unopenable
        );
        println!(
            "  Pages: {} total, {} with text, {} image-only/empty",
            st.pages_total, st.pages_with_text, st.pages_no_text
        );
        println!("  Lines: {}", st.lines_total);
        println!(
            "  A. Font dict matched: {}/{} ({:.1}%), Windows exact font matched: {}/{} ({:.1}%)",
            st.a_font_dict_matched,
            st.lines_total,
            st.a_font_dict_matched as f64 * 100.0 / st.lines_total.max(1) as f64,
            st.a_windows_exact_matched,
            st.lines_total,
            st.a_windows_exact_matched as f64 * 100.0 / st.lines_total.max(1) as f64,
        );
        println!("  A. Subtypes: {:?}", st.a_subtypes);
        println!("  A. Encodings: {:?}", st.a_encodings);
        println!(
            "  B. Fits original font: {}/{} ({:.1}%), needs new subset: {}/{} ({:.1}%)",
            st.b_fits_original_font,
            st.lines_total,
            st.b_fits_original_font as f64 * 100.0 / st.lines_total.max(1) as f64,
            st.b_needs_new_subset,
            st.lines_total,
            st.b_needs_new_subset as f64 * 100.0 / st.lines_total.max(1) as f64,
        );
        println!(
            "  C. Edit succeeded: {}/{} (verified searchable: {}), refused: {:?}, failed: {:?}",
            st.c_edit_ok, st.lines_total, st.c_edit_ok_verified, st.c_refused, st.c_failed
        );
        st
    };

    let corpus_stats = measure_set(
        "tests/corpus (benign + malicious + malformed)",
        &corpus_files,
    );
    let synth_stats = measure_set("generated synthetic samples", &synthetic_samples);

    assert!(corpus_stats.docs_opened >= 50);
    assert!(corpus_stats.lines_total >= 100);
    assert_eq!(
        corpus_stats.c_edit_ok, corpus_stats.c_edit_ok_verified,
        "every succeeded corpus edit must be verified searchable: {:?}",
        corpus_stats.c_failed
    );
    assert!(
        corpus_stats.c_failed.is_empty(),
        "unexpected corpus edit failures: {:?}",
        corpus_stats.c_failed
    );
    assert_eq!(synth_stats.c_edit_ok, synth_stats.c_edit_ok_verified);
    assert!(
        synth_stats.c_failed.is_empty(),
        "unexpected synthetic edit failures: {:?}",
        synth_stats.c_failed
    );
}
