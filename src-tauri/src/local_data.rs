//! The app's own files in its local data folder (docs/architecture/local-data.md): the recent
//! files list (#73) and the settings (B2-12). Read with a size limit, since a file there may not
//! be one the app wrote; written whole through a temporary file, so that a crash never leaves
//! half a file.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

/// The content of `file`, or `None` if it is missing, unreadable or larger than `max_bytes`.
pub fn read(file: &Path, max_bytes: u64) -> Option<Vec<u8>> {
    let mut content = Vec::new();
    File::open(file)
        .ok()?
        .take(max_bytes + 1)
        .read_to_end(&mut content)
        .ok()?;
    (content.len() as u64 <= max_bytes).then_some(content)
}

/// Replaces `file` with `content`: the new content goes to a temporary file next to it, which
/// then takes its place. On failure the old file is left as it was.
pub fn write(file: &Path, content: &[u8]) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    let temporary = file.with_extension("json.tmp");
    let written = fs::write(&temporary, content).and_then(|()| fs::rename(&temporary, file));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Deletes `file` if it exists (best effort: nothing is left to protect if it cannot be).
pub fn remove(file: &Path) {
    let _ = fs::remove_file(file);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_whole_files_and_reads_them_back_within_a_limit() {
        let folder =
            std::env::temp_dir().join(format!("pdf-reader-local-data-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let file = folder.join("nested").join("data.json");

        write(&file, b"{\"a\":1}").unwrap();
        assert_eq!(read(&file, 100).as_deref(), Some(&b"{\"a\":1}"[..]));
        assert_eq!(read(&file, 3), None, "larger than the limit");
        assert!(!file.with_extension("json.tmp").exists());

        write(&file, b"{}").unwrap();
        assert_eq!(read(&file, 100).as_deref(), Some(&b"{}"[..]));
        remove(&file);
        assert_eq!(read(&file, 100), None);
        remove(&file);
        let _ = fs::remove_dir_all(folder);
    }
}
