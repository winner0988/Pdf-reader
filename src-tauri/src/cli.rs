//! Command-line handling:
//! - `pdf-reader.exe <file.pdf>...` opens the files at start-up, and a second launch hands its
//!   files to the running window (file associations, MVP-14).
//! - `pdf-reader.exe --create-pdf-from <image>...` hands image paths to the PDF creation flow
//!   (Explorer context menu, B5-13, #450) rather than opening them as PDF documents.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Command-line flag after which non-option arguments are treated as images for PDF creation.
pub const CREATE_PDF_FROM_FLAG: &str = "--create-pdf-from";

/// Parsed command-line arguments separated into documents to open and images to convert.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Files to open as document tabs.
    pub documents: Vec<PathBuf>,
    /// Files passed after `--create-pdf-from` to turn into a new PDF.
    pub create_from: Vec<PathBuf>,
}

impl Launch {
    /// Resolves relative paths against `cwd`, as a second launch's working directory may differ
    /// from the running instance's. Absolute and UNC paths are returned unchanged by `Path::join`.
    pub fn resolve_relative(self, cwd: &Path) -> Self {
        Self {
            documents: self
                .documents
                .into_iter()
                .map(|path| cwd.join(path))
                .collect(),
            create_from: self
                .create_from
                .into_iter()
                .map(|path| cwd.join(path))
                .collect(),
        }
    }
}

/// Parses command-line arguments after the executable name.
///
/// Before `--create-pdf-from`, every non-option argument is a document to open (unchanged from
/// previous behaviour). Once `--create-pdf-from` appears, every subsequent non-option argument is
/// collected into `create_from` and never added to `documents`.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Launch {
    let mut launch = Launch::default();
    let mut create_mode = false;
    for arg in args.into_iter().skip(1) {
        let text = arg.to_string_lossy();
        if text == CREATE_PDF_FROM_FLAG {
            create_mode = true;
            continue;
        }
        if text.starts_with('-') {
            continue;
        }
        let path = PathBuf::from(arg);
        if create_mode {
            launch.create_from.push(path);
        } else {
            launch.documents.push(path);
        }
    }
    launch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Launch {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_argument_opens_nothing() {
        assert_eq!(parse_args(&["pdf-reader.exe"]), Launch::default());
        assert_eq!(parse_args(&[]), Launch::default());
    }

    #[test]
    fn without_flag_every_file_is_a_document() {
        assert_eq!(
            parse_args(&["pdf-reader.exe", r"C:\Docs\報告 2026.pdf"]),
            Launch {
                documents: vec![PathBuf::from(r"C:\Docs\報告 2026.pdf")],
                create_from: Vec::new(),
            }
        );
        assert_eq!(
            parse_args(&["pdf-reader.exe", "a.pdf", "b.pdf"]),
            Launch {
                documents: vec![PathBuf::from("a.pdf"), PathBuf::from("b.pdf")],
                create_from: Vec::new(),
            }
        );
    }

    #[test]
    fn flag_without_files_produces_empty_lists() {
        assert_eq!(
            parse_args(&["pdf-reader.exe", "--create-pdf-from"]),
            Launch::default()
        );
    }

    #[test]
    fn flag_with_multiple_files_routes_all_to_create_from() {
        assert_eq!(
            parse_args(&[
                "pdf-reader.exe",
                "--create-pdf-from",
                "a.png",
                "b.jpg",
                "c.tif"
            ]),
            Launch {
                documents: Vec::new(),
                create_from: vec![
                    PathBuf::from("a.png"),
                    PathBuf::from("b.jpg"),
                    PathBuf::from("c.tif"),
                ],
            }
        );
    }

    #[test]
    fn documents_before_flag_stay_documents_and_files_after_go_to_create_from() {
        assert_eq!(
            parse_args(&[
                "pdf-reader.exe",
                "doc1.pdf",
                "doc2.pdf",
                "--create-pdf-from",
                "img1.png",
                "img2.jpg",
            ]),
            Launch {
                documents: vec![PathBuf::from("doc1.pdf"), PathBuf::from("doc2.pdf")],
                create_from: vec![PathBuf::from("img1.png"), PathBuf::from("img2.jpg")],
            }
        );
    }

    #[test]
    fn paths_with_spaces_and_chinese_characters_are_preserved() {
        let p1 = r"C:\我的圖片\旅遊 相簿\第 1 張.png";
        let p2 = r"\\server\共享資料夾\掃描 檔.JPG";
        assert_eq!(
            parse_args(&["pdf-reader.exe", "--create-pdf-from", p1, p2]),
            Launch {
                documents: Vec::new(),
                create_from: vec![PathBuf::from(p1), PathBuf::from(p2)],
            }
        );
    }

    #[test]
    fn dash_prefixed_options_are_skipped_in_both_modes() {
        assert_eq!(
            parse_args(&["pdf-reader.exe", "--flag", "a.pdf"]),
            Launch {
                documents: vec![PathBuf::from("a.pdf")],
                create_from: Vec::new(),
            }
        );
        assert_eq!(parse_args(&["pdf-reader.exe", "-x"]), Launch::default());
        assert_eq!(
            parse_args(&[
                "pdf-reader.exe",
                "--create-pdf-from-other",
                "a.pdf",
                "--create-pdf-from",
                "-x",
                "--unknown",
                "b.png",
            ]),
            Launch {
                documents: vec![PathBuf::from("a.pdf")],
                create_from: vec![PathBuf::from("b.png")],
            }
        );
    }

    #[test]
    fn resolve_relative_expands_relative_paths_and_keeps_absolute_paths() {
        let launch = parse_args(&[
            "pdf-reader.exe",
            "rel.pdf",
            r"D:\abs.pdf",
            "--create-pdf-from",
            r"sub\pic.png",
            r"C:\Photos\pic2.jpg",
        ])
        .resolve_relative(Path::new(r"C:\Work"));

        assert_eq!(
            launch.documents,
            vec![
                PathBuf::from(r"C:\Work\rel.pdf"),
                PathBuf::from(r"D:\abs.pdf")
            ]
        );
        assert_eq!(
            launch.create_from,
            vec![
                PathBuf::from(r"C:\Work\sub\pic.png"),
                PathBuf::from(r"C:\Photos\pic2.jpg"),
            ]
        );
    }
}
