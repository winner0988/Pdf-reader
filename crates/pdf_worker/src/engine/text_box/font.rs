//! Reading the font of a text box, which is not trusted, and cutting it to the glyphs of the words:
//! the first half of making a text box. Only `skrifa` and `subsetter` read the bytes, which are
//! memory safe; what comes out is a model (`TextBoxModel`) that holds a subset the box is set from.

use std::collections::BTreeMap;

use ipc_contract::limits::{MAX_TEXT_BOX_BYTES, MAX_TEXT_BOX_SUBSET_BYTES};
use ipc_contract::types::FontRefusal;
use ipc_contract::validate::Validate;
use ipc_contract::worker::{ModelGlyph, TextBoxModel};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::raw::types::Tag;
use skrifa::string::StringId;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use subsetter::{GlyphRemapper, subset};

use super::layout::needs_layout;
use super::{EngineError, TextBoxFont};

/// OS/2 `fsType` (OpenType specification): a restricted licence, preview and print only, no
/// subsetting, bitmaps only.
const RESTRICTED: u16 = 0x0002;
const PREVIEW_AND_PRINT: u16 = 0x0004;
const NO_SUBSETTING: u16 = 0x0100;
const BITMAP_ONLY: u16 = 0x0200;

/// A font that may be used, and what its licence says.
struct Face<'a> {
    font: FontRef<'a>,
    view_only: bool,
    per_em: f32,
}

fn has(font: &FontRef, tag: &[u8; 4]) -> bool {
    font.data_for_tag(Tag::new(tag)).is_some()
}

/// Opens the font `font` names, and says no if the first version cannot set text in it.
fn open_face<'a>(font: &TextBoxFont<'a>) -> Result<Face<'a>, FontRefusal> {
    let face = FontRef::from_index(font.bytes, font.face).map_err(|_| FontRefusal::NotAFont)?;
    if has(&face, b"fvar") {
        return Err(FontRefusal::Variable);
    }
    if [b"COLR", b"CBDT", b"sbix", b"SVG "]
        .iter()
        .any(|tag| has(&face, tag))
    {
        return Err(FontRefusal::Colour);
    }
    if !(has(&face, b"glyf") && has(&face, b"loca")) {
        return Err(FontRefusal::NotTrueType);
    }
    let fs_type = face.os2().map_or(0, |os2| os2.fs_type());
    if fs_type & (RESTRICTED | NO_SUBSETTING | BITMAP_ONLY) != 0 {
        return Err(FontRefusal::NotEmbeddable);
    }
    let per_em = f32::from(
        face.head()
            .map_err(|_| FontRefusal::NotAFont)?
            .units_per_em(),
    );
    if per_em < 16.0 {
        return Err(FontRefusal::NotAFont);
    }
    Ok(Face {
        font: face,
        view_only: fs_type & PREVIEW_AND_PRINT != 0,
        per_em,
    })
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

/// The font's PostScript name as a PDF name allows it: letters, digits and hyphens.
fn base_name(face: &Face) -> String {
    let name: String = face
        .font
        .localized_strings(StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|name| name.chars().collect())
        .unwrap_or_default();
    let plain: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(40)
        .collect();
    if plain.is_empty() {
        "TextBox".to_owned()
    } else {
        plain
    }
}

/// In thousandths of an em.
fn thousandths(units: f32, per_em: f32) -> i32 {
    (units * 1000.0 / per_em).round() as i32
}

/// Makes the model of `text` in `font`: the font read, its licence checked, a glyph found for
/// each character, and the font cut to those glyphs. A font that cannot be used, or that lacks a
/// glyph, makes no model (see [`FontRefusal`]); text that needs a script the worker does not set
/// is refused too.
pub fn prepare_text_box(font: &TextBoxFont, text: &str) -> Result<TextBoxModel, EngineError> {
    if text.trim().is_empty() || text.len() > MAX_TEXT_BOX_BYTES {
        return Err(EngineError::InvalidEdit("the words of a text box"));
    }
    if text.chars().any(needs_layout) {
        return Err(EngineError::UnsupportedText);
    }
    let face = open_face(font).map_err(EngineError::Font)?;
    let charmap = face.font.charmap();
    let metrics = face
        .font
        .glyph_metrics(Size::unscaled(), LocationRef::default());

    // The glyph of each character, and a number for it in the subset, in the order they appear.
    let mut remapper = GlyphRemapper::new();
    remapper.remap(0);
    let mut glyphs: Vec<ModelGlyph> = Vec::new();
    let mut by_char: BTreeMap<char, ()> = BTreeMap::new();
    for ch in text.chars().filter(|&ch| ch != '\n') {
        if by_char.insert(ch, ()).is_some() {
            continue;
        }
        let id = charmap
            .map(ch)
            .map(|glyph| glyph.to_u32())
            .filter(|&glyph| glyph != 0 && glyph <= u32::from(u16::MAX))
            .ok_or(EngineError::Font(FontRefusal::MissingGlyphs))? as u16;
        let cid = remapper.remap(id);
        let advance = metrics
            .advance_width(GlyphId::new(u32::from(id)))
            .unwrap_or(0.0);
        glyphs.push(ModelGlyph {
            ch,
            cid,
            advance: thousandths(advance, face.per_em).clamp(0, 5_000) as u16,
        });
    }
    let subset = subset(font.bytes, font.face, &remapper)
        .map_err(|_| EngineError::Font(FontRefusal::NotAFont))?;
    if subset.len() > MAX_TEXT_BOX_SUBSET_BYTES {
        return Err(EngineError::SubsetTooLarge);
    }

    let font_metrics = face.font.metrics(Size::unscaled(), LocationRef::default());
    let head = face
        .font
        .head()
        .map_err(|_| EngineError::Font(FontRefusal::NotAFont))?;
    let step = thousandths(
        font_metrics.ascent - font_metrics.descent + font_metrics.leading,
        face.per_em,
    )
    .clamp(1_150, 1_600);
    let model = TextBoxModel {
        text: text.to_owned(),
        font: font.name.to_owned(),
        face: font.face,
        view_only: face.view_only,
        base: format!("{}+{}", tag(&subset), base_name(&face)),
        ascent: thousandths(font_metrics.ascent, face.per_em),
        descent: thousandths(font_metrics.descent, face.per_em),
        cap_height: thousandths(
            font_metrics.cap_height.unwrap_or(font_metrics.ascent),
            face.per_em,
        ),
        step,
        bbox: [
            thousandths(f32::from(head.x_min()), face.per_em),
            thousandths(f32::from(head.y_min()), face.per_em),
            thousandths(f32::from(head.x_max()), face.per_em),
            thousandths(f32::from(head.y_max()), face.per_em),
        ],
        glyphs,
        subset,
    };
    // What the main process will check of it. A model it would refuse is of a font that cannot be
    // used here (a name or measures out of the ranges of a text box).
    model
        .validate()
        .map_err(|_| EngineError::Font(FontRefusal::NotAFont))?;
    // Setting the box cuts the subset once more (`clean_subset`). A damaged font can give a
    // subset that `skrifa` cannot read back; that is said now, to the dialog, and not when the
    // box is put down (found by the font fuzz target, B2-31).
    if clean_subset(&model).is_none() {
        return Err(EngineError::Font(FontRefusal::NotAFont));
    }
    Ok(model)
}

/// The subset of `model`, cut again from itself: what is embedded is what `subsetter` writes from
/// bytes `skrifa` could read, also when the model comes from a file on disk and not from
/// [`prepare_text_box`] a moment ago. `None` if it is no font to cut.
pub(super) fn clean_subset(model: &TextBoxModel) -> Option<Vec<u8>> {
    let font = FontRef::new(&model.subset).ok()?;
    if !(has(&font, b"glyf") && has(&font, b"loca") && has(&font, b"head")) {
        return None;
    }
    let count = font.maxp().ok()?.num_glyphs();
    // Every glyph the words use is in it.
    if model.glyphs.iter().any(|glyph| glyph.cid >= count) {
        return None;
    }
    let mut keep = GlyphRemapper::new();
    for id in 0..count {
        keep.remap(id);
    }
    let cut = subset(&model.subset, 0, &keep).ok()?;
    (cut.len() <= MAX_TEXT_BOX_SUBSET_BYTES).then_some(cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_of_a_subset_is_six_capitals_that_follow_the_bytes() {
        let one = tag(b"one");
        assert_eq!(one.len(), 6);
        assert!(one.chars().all(|c| c.is_ascii_uppercase()));
        assert_eq!(one, tag(b"one"));
        assert_ne!(one, tag(b"two"));
    }

    #[test]
    fn measures_are_in_thousandths_of_an_em_whatever_the_units_of_the_font() {
        assert_eq!(thousandths(512.0, 1024.0), 500);
        assert_eq!(thousandths(1000.0, 1000.0), 1000);
        assert_eq!(thousandths(-210.0, 2048.0), -103);
    }
}
