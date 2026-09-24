//! Command-line handling: `pdf-reader.exe <file.pdf>` opens the file at start-up (and prepares
//! for file associations later).

use std::ffi::OsString;
use std::path::PathBuf;

/// The document named on the command line: the first argument after the program name that is
/// not an option. Further documents are ignored (one document per window).
pub fn document_argument(args: impl IntoIterator<Item = OsString>) -> Option<PathBuf> {
    args.into_iter()
        .skip(1)
        .find(|arg| !arg.to_string_lossy().starts_with('-'))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<PathBuf> {
        document_argument(args.iter().map(OsString::from))
    }

    #[test]
    fn no_argument_opens_nothing() {
        assert_eq!(parse(&["pdf-reader.exe"]), None);
        assert_eq!(parse(&[]), None);
    }

    #[test]
    fn the_first_file_is_opened() {
        assert_eq!(
            parse(&["pdf-reader.exe", r"C:\Docs\報告 2026.pdf"]),
            Some(PathBuf::from(r"C:\Docs\報告 2026.pdf"))
        );
        assert_eq!(
            parse(&["pdf-reader.exe", "a.pdf", "b.pdf"]),
            Some(PathBuf::from("a.pdf"))
        );
    }

    #[test]
    fn options_are_skipped() {
        assert_eq!(
            parse(&["pdf-reader.exe", "--flag", "a.pdf"]),
            Some(PathBuf::from("a.pdf"))
        );
        assert_eq!(parse(&["pdf-reader.exe", "-x"]), None);
    }
}
