//! Writing a document to its file (B2-02, ADR 0013). The worker writes through a write-only
//! handle into a new temporary file next to the destination; only once that file is complete and
//! looks like a PDF does it take the destination's place. A failure at any step leaves the
//! destination as it was, and the temporary file is removed.

// ReplaceFileW and MoveFileExW; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ipc_contract::types::{ErrorCode, IpcError};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW,
};

/// Every PDF starts with this (the header), and a complete one ends with the other.
const PDF_HEADER: &[u8] = b"%PDF-";
const PDF_END: &[u8] = b"%%EOF";
/// How far from the end of a file its end marker may be (it may be followed by white space).
const END_SEARCH_BYTES: u64 = 1_024;

/// `ReplaceFileW`: the replacement could not take the destination's name, which now belongs to
/// nothing; the original is under the backup name (winerror.h).
const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1177;

/// What a file was when the app last read or wrote it. Another program changing it since then
/// shows in its size or its modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    len: u64,
    modified: SystemTime,
}

impl FileIdentity {
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }

    /// The size, and the modification time since the Unix epoch: to keep in a file (B2-13).
    pub fn parts(self) -> Option<(u64, Duration)> {
        Some((self.len, self.modified.duration_since(UNIX_EPOCH).ok()?))
    }

    /// `None` for a time the system cannot represent.
    pub fn from_parts(len: u64, since_epoch: Duration) -> Option<Self> {
        Some(Self {
            len,
            modified: UNIX_EPOCH.checked_add(since_epoch)?,
        })
    }
}

/// Refuses a destination that is marked read-only; any other problem shows when writing.
pub fn check_writable(destination: &Path) -> Result<(), IpcError> {
    match fs::metadata(destination) {
        Ok(metadata) if metadata.permissions().readonly() => Err(IpcError {
            code: ErrorCode::ReadOnly,
            message: "the file is read-only".to_owned(),
        }),
        _ => Ok(()),
    }
}

/// A new file next to a destination, for the worker to write the document into. Removed when
/// dropped, unless it took the destination's place.
pub struct Temporary {
    /// Empty once the file has become the destination.
    path: PathBuf,
    file: Option<File>,
}

impl Temporary {
    /// Creates `<destination's name>.<random>.tmp` in the destination's folder: always a new
    /// file, so nothing of the user's is overwritten, and on the same volume, so that it can
    /// replace the destination in one step.
    pub fn new(destination: &Path) -> Result<Self, IpcError> {
        let (Some(folder), Some(name)) = (destination.parent(), destination.file_name()) else {
            return Err(IpcError {
                code: ErrorCode::InvalidArgument,
                message: "not a file path".to_owned(),
            });
        };
        let mut error = io::Error::from(io::ErrorKind::AlreadyExists);
        for _ in 0..16 {
            let path = folder.join(format!("{}.{}.tmp", name.to_string_lossy(), random_hex()?));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(failed) if failed.kind() == io::ErrorKind::AlreadyExists => error = failed,
                Err(failed) => return Err(write_error(&failed)),
            }
        }
        Err(write_error(&error))
    }

    /// The file, open for writing (the worker gets a write-only duplicate) and reading.
    pub fn file(&self) -> &File {
        self.file.as_ref().expect("open until replaced")
    }

    /// Makes sure the `bytes` the worker reports writing are on disk, and that they look like a
    /// whole PDF: the header first and the end marker last. The file's content is not parsed.
    pub fn check(&mut self, bytes: u64) -> Result<(), IpcError> {
        let file = self.file.as_mut().expect("open until replaced");
        file.sync_all().map_err(|error| write_error(&error))?;
        let len = file.metadata().map_err(|error| write_error(&error))?.len();
        let incomplete = || IpcError {
            code: ErrorCode::Unwritable,
            message: "the written file is not a complete PDF".to_owned(),
        };
        if len != bytes || len < (PDF_HEADER.len() + PDF_END.len()) as u64 {
            return Err(incomplete());
        }
        let mut head = [0; PDF_HEADER.len()];
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.read_exact(&mut head))
            .map_err(|error| write_error(&error))?;
        let tail_len = len.min(END_SEARCH_BYTES);
        let mut tail = vec![0; tail_len as usize];
        file.seek(SeekFrom::Start(len - tail_len))
            .and_then(|_| file.read_exact(&mut tail))
            .map_err(|error| write_error(&error))?;
        if head != PDF_HEADER || !tail.trim_ascii_end().ends_with(PDF_END) {
            return Err(incomplete());
        }
        Ok(())
    }

    /// Puts the file in `destination`'s place. An existing destination is replaced with
    /// `ReplaceFileW`, which keeps its attributes and permissions; if that fails, the
    /// destination is as it was.
    pub fn replace(mut self, destination: &Path) -> Result<(), IpcError> {
        // Closed first: a file cannot be renamed while this process has it open.
        drop(self.file.take());
        replace_file(&self.path, destination).map_err(|error| write_error(&error))?;
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn random_hex() -> Result<String, IpcError> {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).map_err(|_| IpcError {
        code: ErrorCode::Internal,
        message: "no randomness for a temporary file name".to_owned(),
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Paths of this many UTF-16 units or more are given to the raw Windows calls below in their
/// extended-length form. `MAX_PATH` is 260 including the terminating NUL, and the backup's name is
/// 13 units longer than the destination's: a destination shorter than this keeps both names under
/// it.
const EXTENDED_FROM: usize = 240;

/// A path as `MoveFileExW` and `ReplaceFileW` take it, without the terminating NUL. The standard
/// library gives its own file functions the `\\?\` prefix when a path needs it, but these two take
/// what they are given and fail beyond `MAX_PATH`: the manifest of this program does not ask for
/// long paths and Windows does not enable them by default (#207). A long path is made absolute and
/// normalised, which the prefix switches off, and gets the prefix: `\\?\C:\...`, or
/// `\\?\UNC\server\share\...` on a network share. A short path is left as it is.
fn extended(path: &Path) -> Vec<u16> {
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if units.len() < EXTENDED_FROM {
        return units;
    }
    let Ok(absolute) = std::path::absolute(path) else {
        return units;
    };
    let absolute: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let starts = |prefix: &str| absolute.starts_with(&prefix.encode_utf16().collect::<Vec<u16>>());
    let with = |prefix: &str, rest: &[u16]| -> Vec<u16> {
        prefix.encode_utf16().chain(rest.iter().copied()).collect()
    };
    if starts(r"\\?\") || starts(r"\\.\") {
        absolute
    } else if starts(r"\\") {
        with(r"\\?\UNC\", &absolute[2..])
    } else if absolute.get(1) == Some(&u16::from(b':')) {
        with(r"\\?\", &absolute)
    } else {
        absolute
    }
}

fn wide(path: &Path) -> Vec<u16> {
    let mut units = extended(path);
    units.push(0);
    units
}

/// Moves `file` to `destination`. An existing destination is first renamed to a backup name,
/// then removed once `file` has its name: without a backup, `ReplaceFileW` can fail with the
/// original already gone (ERROR_UNABLE_TO_MOVE_REPLACEMENT). If the original was moved aside
/// and `file` still could not take its place, the original is put back.
fn replace_file(file: &Path, destination: &Path) -> io::Result<()> {
    if fs::symlink_metadata(destination).is_err() {
        // SAFETY: NUL-terminated paths that outlive the call. Without MOVEFILE_REPLACE_EXISTING,
        // a destination that appeared meanwhile is not overwritten.
        let moved = unsafe {
            MoveFileExW(
                wide(file).as_ptr(),
                wide(destination).as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
        return if moved != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        };
    }
    let backup = destination.with_file_name(format!(
        "{}.{}.bak",
        destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        random_hex().map_err(|_| io::Error::other("no randomness for a backup file name"))?
    ));
    // SAFETY: NUL-terminated paths that outlive the call; no exclusion or reserved pointers.
    let replaced = unsafe {
        ReplaceFileW(
            wide(destination).as_ptr(),
            wide(file).as_ptr(),
            wide(&backup).as_ptr(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced != 0 {
        // The replaced original, which the user chose to overwrite.
        let _ = fs::remove_file(&backup);
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2) {
        // SAFETY: as above.
        unsafe {
            MoveFileExW(
                wide(&backup).as_ptr(),
                wide(destination).as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
    }
    Err(error)
}

/// What the frontend is told when writing fails. The message never has a path.
pub fn write_error(error: &io::Error) -> IpcError {
    // winerror.h
    const ACCESS_DENIED: i32 = 5;
    const WRITE_PROTECT: i32 = 19;
    const SHARING_VIOLATION: i32 = 32;
    const LOCK_VIOLATION: i32 = 33;
    const HANDLE_DISK_FULL: i32 = 39;
    const DISK_FULL: i32 = 112;
    const UNABLE_TO_REMOVE_REPLACED: i32 = 1175;
    let code = match error.raw_os_error() {
        Some(ACCESS_DENIED | WRITE_PROTECT) => ErrorCode::ReadOnly,
        Some(SHARING_VIOLATION | LOCK_VIOLATION | UNABLE_TO_REMOVE_REPLACED) => {
            ErrorCode::FileInUse
        }
        Some(HANDLE_DISK_FULL | DISK_FULL) => ErrorCode::DiskFull,
        _ if error.kind() == io::ErrorKind::StorageFull => ErrorCode::DiskFull,
        _ => ErrorCode::Unwritable,
    };
    IpcError {
        code,
        message: format!("the file could not be written ({:?})", error.kind()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// A fresh folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pdf-reader-saving-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn written(destination: &Path, content: &[u8]) -> Temporary {
        let mut temporary = Temporary::new(destination).unwrap();
        temporary.file.as_mut().unwrap().write_all(content).unwrap();
        temporary
    }

    const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<< >>\nendobj\n%%EOF\n";

    #[test]
    fn a_checked_file_replaces_the_destination_and_nothing_is_left_over() {
        let dir = folder("replace");
        let destination = dir.join("報告.pdf");
        fs::write(&destination, b"%PDF-1.4 the original %%EOF").unwrap();
        let mut temporary = written(&destination, PDF);
        temporary.check(PDF.len() as u64).unwrap();
        temporary.replace(&destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), PDF);
        assert_eq!(names(&dir), ["報告.pdf"]);

        // A new destination is simply created.
        let copy = dir.join("copy.pdf");
        let mut temporary = written(&copy, PDF);
        temporary.check(PDF.len() as u64).unwrap();
        temporary.replace(&copy).unwrap();
        assert_eq!(names(&dir), ["copy.pdf", "報告.pdf"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A folder for one test whose path is longer than `MAX_PATH`: the root, and the folder in it.
    fn long_folder(name: &str) -> (PathBuf, PathBuf) {
        let root = folder(name);
        let mut dir = root.clone();
        for index in 0..7 {
            dir.push(format!("資料夾-{index}-{}", "x".repeat(32)));
        }
        fs::create_dir_all(&dir).unwrap();
        assert!(dir.as_os_str().encode_wide().count() > 300);
        (root, dir)
    }

    #[test]
    fn a_path_longer_than_max_path_is_replaced_and_created_too() {
        let (root, dir) = long_folder("long");
        let destination = dir.join("報告.pdf");
        fs::write(&destination, b"%PDF-1.4 the original %%EOF").unwrap();
        let mut temporary = written(&destination, PDF);
        temporary.check(PDF.len() as u64).unwrap();
        temporary.replace(&destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), PDF);
        assert_eq!(names(&dir), ["報告.pdf"]);

        let copy = dir.join("copy.pdf");
        let mut temporary = written(&copy, PDF);
        temporary.check(PDF.len() as u64).unwrap();
        temporary.replace(&copy).unwrap();
        assert_eq!(names(&dir), ["copy.pdf", "報告.pdf"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn long_paths_get_the_extended_prefix_and_short_ones_do_not() {
        let text = |path: &str| String::from_utf16(&extended(Path::new(path))).unwrap();
        let long = "x".repeat(300);

        assert_eq!(text(r"C:\a\報告.pdf"), r"C:\a\報告.pdf");
        let drive = format!(r"C:\{long}\報告.pdf");
        assert_eq!(text(&drive), format!(r"\\?\{drive}"));
        let share = format!(r"\\server\share\{long}\a.pdf");
        assert_eq!(text(&share), format!(r"\\?\UNC\server\share\{long}\a.pdf"));
        // Already extended, or a device path: as it is.
        let extended_already = format!(r"\\?\C:\{long}\a.pdf");
        assert_eq!(text(&extended_already), extended_already);
        // The prefix turns normalising off, so it is done first.
        assert_eq!(text(&format!("C:/{long}/../b.pdf")), r"\\?\C:\b.pdf");
    }

    #[test]
    fn an_incomplete_file_is_refused_and_removed() {
        let dir = folder("incomplete");
        let destination = dir.join("a.pdf");
        fs::write(&destination, b"original").unwrap();
        for (content, reported) in [
            (&b"%PDF-1.7\n1 0 obj"[..], 16),
            (PDF, PDF.len() as u64 + 1),
            (&b"not a pdf at all %%EOF"[..], 22),
        ] {
            let mut temporary = written(&destination, content);
            assert_eq!(
                temporary.check(reported).unwrap_err().code,
                ErrorCode::Unwritable
            );
            drop(temporary);
        }
        // The original is untouched and no temporary file is left.
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        assert_eq!(names(&dir), ["a.pdf"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_destination_in_use_is_left_as_it_was() {
        let dir = folder("in-use");
        let destination = dir.join("a.pdf");
        fs::write(&destination, b"original").unwrap();
        // Open without sharing delete access, as a program editing the file would.
        let holder = {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_SHARE_READ: u32 = 1;
            OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .open(&destination)
                .unwrap()
        };
        let mut temporary = written(&destination, PDF);
        temporary.check(PDF.len() as u64).unwrap();
        let error = temporary.replace(&destination).unwrap_err();
        assert_eq!(error.code, ErrorCode::FileInUse);
        drop(holder);
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        assert_eq!(names(&dir), ["a.pdf"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_read_only_file_is_refused_before_anything_is_written() {
        let dir = folder("read-only");
        let destination = dir.join("a.pdf");
        fs::write(&destination, b"original").unwrap();
        let mut permissions = fs::metadata(&destination).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&destination, permissions.clone()).unwrap();
        assert_eq!(
            check_writable(&destination).unwrap_err().code,
            ErrorCode::ReadOnly
        );
        assert!(check_writable(&dir.join("new.pdf")).is_ok());
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&destination, permissions).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn identity_changes_with_the_file() {
        let dir = folder("identity");
        let path = dir.join("a.pdf");
        fs::write(&path, b"one").unwrap();
        let before = FileIdentity::of(&path).unwrap();
        assert_eq!(FileIdentity::of(&path), Some(before));
        fs::write(&path, b"three").unwrap();
        assert_ne!(FileIdentity::of(&path), Some(before));
        assert_eq!(FileIdentity::of(&dir.join("missing.pdf")), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_errors_say_what_went_wrong() {
        let code = |raw| write_error(&io::Error::from_raw_os_error(raw)).code;
        assert_eq!(code(5), ErrorCode::ReadOnly);
        assert_eq!(code(32), ErrorCode::FileInUse);
        assert_eq!(code(112), ErrorCode::DiskFull);
        assert_eq!(code(1_392), ErrorCode::Unwritable);
        assert!(
            !write_error(&io::Error::from_raw_os_error(5))
                .message
                .contains('\\')
        );
    }
}
