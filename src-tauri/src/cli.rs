//! Command-line handling: `pdf-reader.exe <file.pdf>...` opens the files at start-up, and a
//! second launch hands its files to the running window (file associations, MVP-14).

use std::ffi::OsString;
use std::path::PathBuf;

/// The documents named on the command line: every argument after the program name that is not
/// an option. Each one gets a tab.
pub fn document_arguments(args: impl IntoIterator<Item = OsString>) -> Vec<PathBuf> {
    args.into_iter()
        .skip(1)
        .filter(|arg| !arg.to_string_lossy().starts_with('-'))
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Vec<PathBuf> {
        document_arguments(args.iter().map(OsString::from))
    }

    #[test]
    fn no_argument_opens_nothing() {
        assert_eq!(parse(&["pdf-reader.exe"]), Vec::<PathBuf>::new());
        assert_eq!(parse(&[]), Vec::<PathBuf>::new());
    }

    #[test]
    fn every_file_is_opened() {
        assert_eq!(
            parse(&["pdf-reader.exe", r"C:\Docs\報告 2026.pdf"]),
            [PathBuf::from(r"C:\Docs\報告 2026.pdf")]
        );
        assert_eq!(
            parse(&["pdf-reader.exe", "a.pdf", "b.pdf"]),
            [PathBuf::from("a.pdf"), PathBuf::from("b.pdf")]
        );
    }

    #[test]
    fn options_are_skipped() {
        assert_eq!(
            parse(&["pdf-reader.exe", "--flag", "a.pdf"]),
            [PathBuf::from("a.pdf")]
        );
        assert_eq!(parse(&["pdf-reader.exe", "-x"]), Vec::<PathBuf>::new());
    }
}
