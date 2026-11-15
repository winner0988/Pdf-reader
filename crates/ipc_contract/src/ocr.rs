//! What recognising the text of scanned pages (B2-10, ADR 0015) needs from the contract: how big
//! language data may be, how a language is named, and a check of the data's structure.
//!
//! The language data (a Tesseract `.traineddata` file) is read by the main process and sent to
//! the worker as bytes. Tesseract does not defend itself against a file that is not one: an empty
//! buffer, or offsets that run backwards, make it read where it should not or allocate without
//! end. So both sides refuse such a file before it gets near Tesseract: the main process when the
//! user imports one, the worker when it is given one. The check goes as far as the file's table
//! of contents; the model inside is Tesseract's to read, in the worker's sandbox. It also checks
//! entry 0 (the language's settings), accepting only settings used by official language data.

use thiserror::Error;

use crate::limits::{MAX_LANGUAGE_DATA_BYTES, MAX_LANGUAGE_NAME_BYTES};

/// Whether `name` can name a language: a letter, then up to 31 more letters, digits, `_` or `-`
/// (all ASCII). Tesseract names its data `<name>.traineddata`.
pub fn is_language_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_LANGUAGE_NAME_BYTES
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LanguageDataError {
    #[error("the language data is larger than {MAX_LANGUAGE_DATA_BYTES} bytes")]
    TooLarge,
    #[error("not language data of Tesseract: {0}")]
    Malformed(&'static str),
    #[error("the language data has no LSTM model, which is the only kind the app uses")]
    NoLstm,
    #[error("unsupported Tesseract setting in language data: {0}")]
    UnsupportedSetting(&'static str),
}

/// The maximum size of entry 0 (the language's settings): 4096 bytes.
const MAX_CONFIG_BYTES: usize = 4096;
/// The maximum length of a setting line, including line endings: 255 bytes.
const MAX_CONFIG_LINE_BYTES: usize = 255;

/// The 59 (name, value) setting pairs that official Tesseract language data uses.
/// Collected on 2026-11-15 from 491 `.traineddata` files across tesseract-ocr's `tessdata_fast`,
/// `tessdata_best`, and `tessdata` repositories (87 files have settings).
///
/// Two setting names are accepted with any value and therefore omitted from this table:
/// - `tessedit_char_blacklist`: only disables characters from recognition.
/// - `tessedit_load_sublangs`: overridden by the worker with an empty string at init.
pub const ALLOWED_SETTINGS: [(&str, &str); 59] = [
    ("allow_blob_division", "F"),
    ("assume_fixed_pitch_char_segment", "T"),
    ("chop_enable", "0"),
    ("chop_enable", "F"),
    ("classify_cp_cutoff_strength", "2"),
    ("classify_integer_matcher_multiplier", "10"),
    ("classify_integer_matcher_multiplier", "4"),
    ("classify_integer_matcher_multiplier", "6"),
    ("classify_misfit_junk_penalty", "0.125"),
    ("disable_character_fragments", "F"),
    ("edges_children_count_limit", "1000"),
    ("edges_children_count_limit", "10000"),
    ("edges_children_fix", "T"),
    ("edges_max_children_layers", "5"),
    ("edges_max_children_per_outline", "10"),
    ("edges_max_children_per_outline", "20"),
    ("edges_use_new_outline_complexity", "T"),
    ("language_model_ngram_nonmatch_score", "-20"),
    ("language_model_ngram_on", "1"),
    ("language_model_ngram_scale_factor", "0.1"),
    ("language_model_ngram_space_delimited_language", "F"),
    ("language_model_use_sigmoidal_certainty", "T"),
    ("merge_fragments_in_matrix", "F"),
    ("ocr_devanagari_split_strategy", "1"),
    ("pageseg_devanagari_split_strategy", "1"),
    ("preserve_interword_spaces", "1"),
    ("prioritize_division", "T"),
    ("segment_nonalphabetic_script", "1"),
    ("segment_penalty_dict_nonword", "1.0"),
    ("segment_penalty_garbage", "1.0"),
    ("segsearch_max_char_wh_ratio", "1.3"),
    ("segsearch_max_futile_classifications", "10"),
    ("stopper_nondict_certainty_base", "-2"),
    ("tessedit_ocr_engine_mode", "1"),
    ("tessedit_tess_adaption_mode", "0x00"),
    ("tessedit_use_primary_params_model", "1"),
    ("textord_force_make_prop_words", "T"),
    ("textord_max_noise_size", "7"),
    ("textord_no_rejects", "T"),
    ("textord_noise_normratio", "5"),
    ("textord_noise_normratio", "6"),
    ("textord_noise_rejwords", "F"),
    ("textord_noise_rowratio", "20.0"),
    ("textord_noise_sizelimit", "0.1"),
    ("textord_noise_sizelimit", "0.2"),
    ("textord_noise_syfract", "0.6"),
    ("textord_single_height_mode", "T"),
    ("textord_space_size_is_variable", "T"),
    ("textord_tabfind_aligned_gap_fraction", "0.5"),
    ("textord_tabfind_vertical_text_ratio", "0.1"),
    ("textord_tabvector_vertical_box_ratio", "0.3"),
    ("textord_tabvector_vertical_gap_fraction", "1.5"),
    ("textord_use_cjk_fp_model", "T"),
    ("tosp_force_wordbreak_on_punct", "F"),
    ("tosp_old_sp_kn_th_factor", "4.0"),
    ("tosp_old_to_constrain_sp_kn", "T"),
    ("tosp_old_to_method", "T"),
    ("tosp_only_small_gaps_for_kern", "T"),
    ("tosp_use_pre_chopping", "T"),
];

/// Validates entry 0 (language config) against official allowed settings.
fn check_language_config(config: &[u8]) -> Result<(), LanguageDataError> {
    if config.len() > MAX_CONFIG_BYTES {
        return Err(LanguageDataError::UnsupportedSetting(
            "the settings are longer than 4096 bytes",
        ));
    }
    for &b in config {
        if (b < 0x20 && b != b'\t' && b != b'\n' && b != b'\r') || b == 0x7F {
            return Err(LanguageDataError::UnsupportedSetting(
                "a control character is present",
            ));
        }
    }
    let mut remaining = config;
    while !remaining.is_empty() {
        let line_with_lf = match remaining.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                let line = &remaining[..=pos];
                remaining = &remaining[pos + 1..];
                line
            }
            None => {
                let line = remaining;
                remaining = &[];
                line
            }
        };
        if line_with_lf.len() > MAX_CONFIG_LINE_BYTES {
            return Err(LanguageDataError::UnsupportedSetting("a line is too long"));
        }
        if let Some(&first) = line_with_lf.first() {
            if first == b'\r' || first == b'\n' || first == b'#' {
                continue;
            }
        } else {
            continue;
        }
        let mut line = line_with_lf;
        while let Some((&b'\r' | &b'\n', rest)) = line.split_last() {
            line = rest;
        }
        let Some(pos) = line.iter().position(|&b| b == b' ' || b == b'\t') else {
            return Err(LanguageDataError::UnsupportedSetting("a line has no value"));
        };
        if pos == 0 {
            return Err(LanguageDataError::UnsupportedSetting(
                "a line begins with whitespace",
            ));
        }
        let name = &line[..pos];
        let val_start = line[pos..]
            .iter()
            .position(|&b| b != b' ' && b != b'\t')
            .map(|offset| pos + offset)
            .unwrap_or(line.len());
        let val = &line[val_start..];
        if name == b"tessedit_char_blacklist" || name == b"tessedit_load_sublangs" {
            continue;
        }
        let is_allowed = ALLOWED_SETTINGS.iter().any(|&(allowed_name, allowed_val)| {
            name == allowed_name.as_bytes() && val == allowed_val.as_bytes()
        });
        if !is_allowed {
            return Err(LanguageDataError::UnsupportedSetting(
                "an unsupported setting or value",
            ));
        }
    }
    Ok(())
}

/// Tesseract's `TessdataType`: the entries of the table of contents the LSTM engine needs.
const TESSDATA_LSTM: usize = 17;
const TESSDATA_LSTM_UNICHARSET: usize = 21;
const TESSDATA_LSTM_RECODER: usize = 22;

/// Fewest entries a table of contents has for those three (the index of the last, plus one), and
/// most any has: Tesseract itself defines 24 and refuses more than 1 000.
const MIN_ENTRIES: usize = TESSDATA_LSTM_RECODER + 1;
const MAX_ENTRIES: usize = 64;

/// Checks `data` as far as its table of contents: a count of entries (4 bytes, little-endian),
/// then the offset of each (8 bytes each, or -1 for an entry that is not there). The entries lie
/// one after another from the end of the table to the end of the file, so the offsets that are
/// there start at that end and never go backwards or beyond the file; and the three entries of an
/// LSTM model are among them.
pub fn check_language_data(data: &[u8]) -> Result<(), LanguageDataError> {
    if data.len() > MAX_LANGUAGE_DATA_BYTES {
        return Err(LanguageDataError::TooLarge);
    }
    let Some(count) = data.first_chunk::<4>().map(|c| i32::from_le_bytes(*c)) else {
        return Err(LanguageDataError::Malformed("shorter than its header"));
    };
    let count = usize::try_from(count)
        .ok()
        .filter(|count| (MIN_ENTRIES..=MAX_ENTRIES).contains(count))
        .ok_or(LanguageDataError::Malformed("the number of entries"))?;
    let table_end = 4 + 8 * count;
    let table = data
        .get(4..table_end)
        .ok_or(LanguageDataError::Malformed("shorter than its table"))?;
    let (chunks, _) = table.as_chunks::<8>();
    let offsets: Vec<i64> = chunks
        .iter()
        .map(|chunk| i64::from_le_bytes(*chunk))
        .collect();
    let end = i64::try_from(data.len()).expect("at most MAX_LANGUAGE_DATA_BYTES");
    let mut earliest = i64::try_from(table_end).expect("a small table");
    let mut first = true;
    for &offset in offsets.iter().filter(|&&offset| offset != -1) {
        // The first entry begins where the table ends, each next one where the one before began
        // or later (an empty entry has no length), and none after the file ends.
        let begins_well = if first {
            offset == earliest
        } else {
            offset >= earliest
        };
        if !begins_well || offset > end {
            return Err(LanguageDataError::Malformed("the offsets of the entries"));
        }
        first = false;
        earliest = offset;
    }
    let present = |index: usize| offsets.get(index).is_some_and(|&offset| offset != -1);
    if ![
        TESSDATA_LSTM,
        TESSDATA_LSTM_UNICHARSET,
        TESSDATA_LSTM_RECODER,
    ]
    .into_iter()
    .all(present)
    {
        return Err(LanguageDataError::NoLstm);
    }
    if offsets[0] != -1 {
        let start = usize::try_from(offsets[0]).expect("offset >= 0");
        let end = (1..count)
            .find_map(|j| {
                let off = offsets[j];
                if off != -1 {
                    usize::try_from(off).ok()
                } else {
                    None
                }
            })
            .unwrap_or(data.len());
        let config = data
            .get(start..end)
            .ok_or(LanguageDataError::Malformed("entry 0 out of bounds"))?;
        check_language_config(config)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Language data with the entries `present` (their indexes), each `size` bytes of filler.
    fn data(count: usize, present: &[usize], size: usize) -> Vec<u8> {
        let table_end = 4 + 8 * count;
        let mut out = i32::try_from(count).unwrap().to_le_bytes().to_vec();
        let mut at = table_end;
        for index in 0..count {
            let offset: i64 = if present.contains(&index) {
                let here = at;
                at += size;
                i64::try_from(here).unwrap()
            } else {
                -1
            };
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.resize(at, 7);
        out
    }

    const LSTM: [usize; 6] = [17, 18, 19, 20, 21, 22];

    #[test]
    fn language_data_with_a_model_is_accepted() {
        assert_eq!(check_language_data(&data(24, &LSTM, 10)), Ok(()));
        assert_eq!(
            check_language_data(&data(24, &[0, 17, 21, 22, 23], 0)),
            Ok(())
        );
        assert_eq!(check_language_data(&data(23, &[17, 21, 22], 3)), Ok(()));
    }

    #[test]
    fn what_is_not_language_data_is_refused() {
        for bytes in [
            &[][..],
            &[0; 3][..],
            &[0; 100][..],
            &[255; 1000][..],
            b"eng.traineddata",
        ] {
            assert!(matches!(
                check_language_data(bytes),
                Err(LanguageDataError::Malformed(_))
            ));
        }
        // Counts of entries that cannot be.
        for count in [-1, 0, 1, 22, 65, i32::MAX, i32::MIN] {
            let mut bytes = data(24, &LSTM, 10);
            bytes[..4].copy_from_slice(&count.to_le_bytes());
            assert!(
                matches!(
                    check_language_data(&bytes),
                    Err(LanguageDataError::Malformed(_))
                ),
                "{count}"
            );
        }
    }

    #[test]
    fn a_table_that_does_not_fit_in_the_file_is_refused() {
        let bytes = data(24, &LSTM, 10);
        for cut in [4, 5, 100, 4 + 8 * 24 - 1] {
            assert!(
                matches!(
                    check_language_data(&bytes[..cut]),
                    Err(LanguageDataError::Malformed(_))
                ),
                "{cut}"
            );
        }
    }

    #[test]
    fn offsets_that_go_backwards_or_beyond_the_file_are_refused() {
        let table_end: i64 = 4 + 8 * 24;
        let set = |bytes: &mut Vec<u8>, index: usize, offset: i64| {
            bytes[4 + 8 * index..4 + 8 * (index + 1)].copy_from_slice(&offset.to_le_bytes());
        };
        let good = data(24, &LSTM, 10);
        let end = i64::try_from(good.len()).unwrap();
        let first = table_end;
        for (index, offset) in [
            (17, first + 1), // the first entry does not begin where the table ends
            (17, first - 1),
            (19, first + 4), // an entry begins before the one before it
            (19, -2),
            (20, end + 1), // beyond the end
            (21, i64::MAX),
            (22, i64::MIN),
        ] {
            let mut bytes = good.clone();
            set(&mut bytes, index, offset);
            assert!(
                matches!(
                    check_language_data(&bytes),
                    Err(LanguageDataError::Malformed(_))
                ),
                "entry {index} at {offset}"
            );
        }
        // An entry that begins at the very end is empty, which is fine.
        let mut bytes = good;
        set(&mut bytes, 22, end);
        assert_eq!(check_language_data(&bytes), Ok(()));
    }

    #[test]
    fn language_data_without_an_lstm_model_is_refused() {
        // The legacy engine's files: no LSTM entries.
        assert_eq!(
            check_language_data(&data(24, &[0, 1, 2, 3, 4, 5], 10)),
            Err(LanguageDataError::NoLstm)
        );
        for missing in [17, 21, 22] {
            let present: Vec<usize> = LSTM.into_iter().filter(|&i| i != missing).collect();
            assert_eq!(
                check_language_data(&data(24, &present, 10)),
                Err(LanguageDataError::NoLstm),
                "{missing}"
            );
        }
    }

    /// The language data the installer carries (src-tauri/resources/tessdata): tessdata_fast at
    /// commit 87416418657359cb625c412a48b6e1d6d41c29bd, not changed. Replacing them means
    /// changing the README next to them, and this.
    #[test]
    fn the_language_data_of_the_installer_is_the_one_that_was_checked_in_and_passes() {
        use sha2::{Digest, Sha256};
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src-tauri/resources/tessdata");
        for (language, size, sha256) in [
            (
                "eng",
                4_113_088,
                "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2",
            ),
            (
                "chi_tra",
                2_366_642,
                "529c5b5797d64b126065cd55f2bb4c7fd7b15790798091b1ff259941a829330b",
            ),
        ] {
            let path = folder.join(format!("{language}.traineddata"));
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(bytes.len(), size, "{language}");
            let digest: String = Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(digest, sha256, "{language}");
            assert_eq!(check_language_data(&bytes), Ok(()), "{language}");
        }
    }

    #[test]
    fn language_data_over_the_limit_is_refused() {
        let big = vec![0; MAX_LANGUAGE_DATA_BYTES + 1];
        assert_eq!(check_language_data(&big), Err(LanguageDataError::TooLarge));
    }

    #[test]
    fn names_of_languages() {
        for name in ["eng", "chi_tra", "chi_sim_vert", "HanT", "x", "a-b"] {
            assert!(is_language_name(name), "{name}");
        }
        for name in [
            "",
            "1eng",
            "_eng",
            "-x",
            "script/HanT",
            "..",
            "a b",
            "eng.traineddata",
            "中文",
            "e\u{0}ng",
            &"a".repeat(33),
        ] {
            assert!(!is_language_name(name), "{name:?}");
        }
        assert!(is_language_name(&"a".repeat(32)));
    }

    /// Language data with entry 0 (the language's settings) containing `config`, and LSTM models present.
    fn data_with_config(config: &[u8]) -> Vec<u8> {
        let count = 24;
        let table_end = 4 + 8 * count;
        let mut out = i32::try_from(count).unwrap().to_le_bytes().to_vec();
        let mut at = table_end;
        let entry0_offset = i64::try_from(at).unwrap();
        at += config.len();
        out.extend_from_slice(&entry0_offset.to_le_bytes());
        for index in 1..count {
            let offset: i64 = if LSTM.contains(&index) {
                let here = at;
                at += 10;
                i64::try_from(here).unwrap()
            } else {
                -1
            };
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.extend_from_slice(config);
        out.resize(at, 7);
        out
    }

    #[test]
    fn language_data_with_allowed_settings_is_accepted() {
        // Every setting pair in ALLOWED_SETTINGS individually passes.
        for &(name, val) in &ALLOWED_SETTINGS {
            let line = format!("{name} {val}\n");
            assert_eq!(
                check_language_data(&data_with_config(line.as_bytes())),
                Ok(()),
                "{name} {val}"
            );
        }
        // Tabs or multiple spaces between name and value pass.
        assert_eq!(
            check_language_data(&data_with_config(b"chop_enable\t0\n")),
            Ok(())
        );
        assert_eq!(
            check_language_data(&data_with_config(b"chop_enable   0\n")),
            Ok(())
        );
        // Empty lines, comments, CRLF line endings, and no trailing newline pass.
        let mixed = b"# comment\n\r\nchop_enable 0\r\npreserve_interword_spaces 1";
        assert_eq!(check_language_data(&data_with_config(mixed)), Ok(()));
        // tessedit_char_blacklist with non-UTF-8 bytes passes.
        let mut blacklist = b"tessedit_char_blacklist ".to_vec();
        blacklist.extend_from_slice(&[0xA4, 0x40]);
        blacklist.push(b'\n');
        assert_eq!(check_language_data(&data_with_config(&blacklist)), Ok(()));
        // tessedit_load_sublangs with arbitrary value passes.
        assert_eq!(
            check_language_data(&data_with_config(b"tessedit_load_sublangs whatever_lang\n")),
            Ok(())
        );
        // A line of exactly 255 bytes passes.
        let mut line_255 = b"tessedit_load_sublangs ".to_vec();
        line_255.resize(254, b'a');
        line_255.push(b'\n');
        assert_eq!(check_language_data(&data_with_config(&line_255)), Ok(()));
        // Empty entry 0 passes.
        assert_eq!(check_language_data(&data_with_config(&[])), Ok(()));
    }

    #[test]
    fn language_data_with_unsupported_settings_is_refused() {
        let is_unsupported = |config: &[u8]| {
            matches!(
                check_language_data(&data_with_config(config)),
                Err(LanguageDataError::UnsupportedSetting(_))
            )
        };
        // Name not in the list.
        assert!(is_unsupported(b"tessedit_write_params_to_file x\n"));
        // Name in the list, but value not in the list.
        assert!(is_unsupported(b"chop_enable 2\n"));
        // Trailing whitespace after value.
        assert!(is_unsupported(b"chop_enable 0 \n"));
        // Leading space or tab.
        assert!(is_unsupported(b" chop_enable 0\n"));
        assert!(is_unsupported(b"\tchop_enable 0\n"));
        // Only name without value or space.
        assert!(is_unsupported(b"chop_enable\n"));
        assert!(is_unsupported(b"chop_enable"));
        // Control characters: NUL, 0x01, 0x7F.
        assert!(is_unsupported(b"chop_enable \x00\n"));
        assert!(is_unsupported(b"chop_enable \x01\n"));
        assert!(is_unsupported(b"chop_enable \x7F\n"));
        // Line of 256 bytes.
        let mut line_256 = b"tessedit_load_sublangs ".to_vec();
        line_256.resize(255, b'a');
        line_256.push(b'\n');
        assert!(is_unsupported(&line_256));
        // Settings exceeding 4096 bytes.
        let big = vec![b'\n'; 4097];
        assert!(is_unsupported(&big));
    }
}
