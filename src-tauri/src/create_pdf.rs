//! Making a PDF of files (B3-03, docs/architecture/create-pdf.md): which kind of file each one the
//! user picked is, and which of them can go into one PDF. The kind comes from the file's
//! extension. It only tells MuPDF how to read the file: a file that is not what its name says
//! ends in an error from the worker, never in anything worse.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use ipc_contract::limits::MAX_CREATE_SOURCES;
use ipc_contract::types::{CreateKind, ErrorCode, IpcError};

/// Every kind of file, in the order the open dialog lists them.
pub const KINDS: [CreateKind; 9] = [
    CreateKind::Image,
    CreateKind::Html,
    CreateKind::Text,
    CreateKind::Epub,
    CreateKind::Xps,
    CreateKind::Svg,
    CreateKind::Docx,
    CreateKind::Xlsx,
    CreateKind::Pptx,
];

/// The extensions of the files of `kind`, in lower case and without the dot.
pub fn extensions(kind: CreateKind) -> &'static [&'static str] {
    match kind {
        CreateKind::Image => &["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff"],
        CreateKind::Html => &["html", "htm", "xhtml"],
        CreateKind::Text => &["txt"],
        CreateKind::Epub => &["epub"],
        CreateKind::Xps => &["xps", "oxps"],
        CreateKind::Svg => &["svg"],
        CreateKind::Docx => &["docx"],
        CreateKind::Xlsx => &["xlsx"],
        CreateKind::Pptx => &["pptx"],
    }
}

/// The pattern of an open dialog's filter for the files of `kinds`: `*.png;*.jpg;...`.
pub fn pattern(kinds: &[CreateKind]) -> String {
    kinds
        .iter()
        .flat_map(|kind| extensions(*kind))
        .map(|extension| format!("*.{extension}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// The kind of the file at `path` by its extension; `None` for any other extension.
pub fn kind_of(path: &Path) -> Option<CreateKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    KINDS
        .into_iter()
        .find(|kind| extensions(*kind).contains(&extension.as_str()))
}

/// The files the user picked, each with its kind, in the order of the PDF's pages: only pictures
/// go together (any number of them, in the order of their names, numbers compared as numbers:
/// `2.png` before `10.png`), any other kind is the whole PDF and alone.
pub fn sources_of(paths: Vec<PathBuf>) -> Result<Vec<(PathBuf, CreateKind)>, IpcError> {
    let invalid = |message: &str| IpcError {
        code: ErrorCode::InvalidArgument,
        message: message.to_owned(),
    };
    if paths.is_empty() || paths.len() > MAX_CREATE_SOURCES {
        return Err(invalid("no files, or too many"));
    }
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let kind = kind_of(&path).ok_or_else(|| invalid("not a kind of file a PDF is made of"))?;
        sources.push((path, kind));
    }
    if sources.len() > 1 && sources.iter().any(|(_, kind)| *kind != CreateKind::Image) {
        return Err(invalid("only pictures go together"));
    }
    sources.sort_by(|(a, _), (b, _)| natural_order(&name_of(a), &name_of(b)));
    Ok(sources)
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The order of two file names as a person reads them: without regard to case, and a run of
/// digits is a number (`page2` before `page10`).
fn natural_order(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (x, y) = (digits(&mut a), digits(&mut b));
                // More digits is a larger number once the zeros in front are gone.
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(&y));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                a.next();
                b.next();
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// Takes the run of digits `chars` is at, without the zeros in front.
fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut run = String::new();
    while let Some(digit) = chars.next_if(char::is_ascii_digit) {
        run.push(digit);
    }
    run.trim_start_matches('0').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| Path::new("C:\\f").join(name))
            .collect()
    }

    #[test]
    fn a_kind_is_known_by_the_extension_in_any_case() {
        for (name, kind) in [
            ("a.PNG", CreateKind::Image),
            ("a.jpeg", CreateKind::Image),
            ("a.Tif", CreateKind::Image),
            ("a.htm", CreateKind::Html),
            ("a.TXT", CreateKind::Text),
            ("a.epub", CreateKind::Epub),
            ("a.oxps", CreateKind::Xps),
            ("a.svg", CreateKind::Svg),
            ("a.docx", CreateKind::Docx),
            ("a.xlsx", CreateKind::Xlsx),
            ("a.pptx", CreateKind::Pptx),
            ("b.c.d.docx", CreateKind::Docx),
        ] {
            assert_eq!(kind_of(Path::new(name)), Some(kind), "{name}");
        }
        for name in [
            "a.pdf",
            "a.doc",
            "a.exe",
            "a",
            ".docx",
            "docx",
            "a.docx.exe",
        ] {
            assert_eq!(kind_of(Path::new(name)), None, "{name}");
        }
    }

    #[test]
    fn every_kind_has_extensions_and_none_has_another_kinds() {
        let mut seen = std::collections::HashSet::new();
        for kind in KINDS {
            assert!(!extensions(kind).is_empty(), "{kind:?}");
            for extension in extensions(kind) {
                assert!(seen.insert(*extension), "{extension} is of two kinds");
                assert_eq!(*extension, extension.to_ascii_lowercase());
                assert_eq!(
                    kind_of(Path::new(&format!("x.{extension}"))),
                    Some(kind),
                    "{extension}"
                );
            }
        }
        assert_eq!(
            pattern(&[CreateKind::Text, CreateKind::Epub]),
            "*.txt;*.epub"
        );
    }

    #[test]
    fn pictures_go_together_in_the_order_of_their_names_and_other_files_alone() {
        let sources =
            sources_of(paths(&["10.png", "page 01b.gif", "2.jpg", "Page 1.png"])).unwrap();
        let names: Vec<String> = sources.iter().map(|(path, _)| name_of(path)).collect();
        assert_eq!(names, ["2.jpg", "10.png", "Page 1.png", "page 01b.gif"]);
        assert!(sources.iter().all(|(_, kind)| *kind == CreateKind::Image));

        // One file of any kind is a PDF of its own.
        let [(path, kind)] = &sources_of(paths(&["report.DOCX"])).unwrap()[..] else {
            panic!("one source")
        };
        assert_eq!(
            (name_of(path).as_str(), *kind),
            ("report.DOCX", CreateKind::Docx)
        );
        assert_eq!(sources_of(paths(&["a.txt"])).unwrap().len(), 1);

        // Two files that are not pictures, or a picture and something else: no.
        for names in [
            &["a.docx", "b.docx"][..],
            &["a.txt", "b.html"],
            &["a.png", "b.txt"],
            &["a.png", "b.png", "c.epub"],
        ] {
            assert_eq!(
                sources_of(paths(names)).unwrap_err().code,
                ErrorCode::InvalidArgument,
                "{names:?}"
            );
        }
        // Nothing, a kind that is not one, and too many.
        assert!(sources_of(Vec::new()).is_err());
        assert!(sources_of(paths(&["a.pdf"])).is_err());
        assert!(sources_of(paths(&["a.png", "b.exe"])).is_err());
        let many: Vec<String> = (0..=MAX_CREATE_SOURCES)
            .map(|n| format!("{n}.png"))
            .collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(sources_of(paths(&many)).is_err());
        assert_eq!(
            sources_of(paths(&many[..MAX_CREATE_SOURCES]))
                .unwrap()
                .len(),
            MAX_CREATE_SOURCES
        );
    }

    #[test]
    fn names_are_compared_as_a_person_reads_them() {
        for (smaller, larger) in [
            ("a2", "a10"),
            ("a02", "a10"),
            ("A", "b"),
            ("a", "ab"),
            ("img 9", "img 10"),
            ("0.png", "1.png"),
            ("a1b", "a1c"),
            ("a", "a1"),
        ] {
            assert_eq!(natural_order(smaller, larger), Ordering::Less, "{smaller}");
            assert_eq!(
                natural_order(larger, smaller),
                Ordering::Greater,
                "{larger}"
            );
        }
        assert_eq!(natural_order("File1", "file1"), Ordering::Equal);
        assert_eq!(natural_order("a007", "a7"), Ordering::Equal);
        assert_eq!(natural_order("", ""), Ordering::Equal);
        // Digits longer than any integer are only compared as text.
        let long = "9".repeat(60);
        assert_eq!(
            natural_order(&format!("a{long}"), &format!("a1{long}")),
            Ordering::Less
        );
    }
}
