//! Saved signature images (B6-02, docs/architecture/signature-images.md).
//!
//! Signatures the user drew, typed, or imported are kept as PNG files in the app's local data
//! folder under `signatures/`, named `<id>.sig` where `<id>` is a decimal `u32`. The file content
//! is the whole PNG encrypted with DPAPI for the current Windows user, with a dedicated entropy string.
//!
//! There is no index file: the folder listing is the source of truth.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ipc_contract::limits::{MAX_SIGNATURE_FILE_BYTES, MAX_SIGNATURE_IMAGES, MAX_STAMP_PNG_BYTES};
use ipc_contract::types::{ErrorCode, IpcError, SignatureImage, SignatureImageId};
use ipc_contract::validate::stamp_png_size;
use zeroize::Zeroizing;

use crate::index_crypto;
use crate::local_data;

/// The folder in the app's local data folder.
pub const FOLDER_NAME: &str = "signatures";

/// Entropy mixed into DPAPI for signature image files (B6-02).
pub const ENTROPY: &[u8] = b"Pdf-reader signature image (B6-02)";

#[derive(Clone)]
pub struct SignatureImages {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    folder: Option<PathBuf>,
    /// Highest ID seen or generated during this execution. Starts at 0.
    highest_seen_id: u32,
}

impl SignatureImages {
    pub fn new(folder: Option<PathBuf>) -> Self {
        let mut inner = Inner {
            folder,
            highest_seen_id: 0,
        };
        inner.refresh_highest_id();
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Lists the saved signature images, ordered by id ascending, at most `MAX_SIGNATURE_IMAGES`.
    pub fn list(&self) -> Result<Vec<SignatureImage>, IpcError> {
        let mut inner = self.inner.lock().unwrap();
        inner.list()
    }

    /// Gets the decrypted PNG bytes of a saved signature image for UI preview.
    pub fn get_png(&self, id: SignatureImageId) -> Result<Vec<u8>, IpcError> {
        let plain = self.get_png_zeroizing(id)?;
        Ok(plain.to_vec())
    }

    /// Gets the decrypted PNG bytes in a `Zeroizing` buffer, cleared on drop.
    pub fn get_png_zeroizing(&self, id: SignatureImageId) -> Result<Zeroizing<Vec<u8>>, IpcError> {
        let inner = self.inner.lock().unwrap();
        let Some(folder) = &inner.folder else {
            return Err(IpcError {
                code: ErrorCode::Unreadable,
                message: "signatures folder is not configured".to_owned(),
            });
        };
        let path = folder.join(format!("{}.sig", id.0));
        let (plain, _, _) = read_signature_file(&path).ok_or_else(|| IpcError {
            code: ErrorCode::Unreadable,
            message: "signature image not found or corrupted".to_owned(),
        })?;
        Ok(plain)
    }

    /// Encrypts and adds a new signature image PNG, returning the updated list.
    pub fn add(&self, png: &[u8]) -> Result<Vec<SignatureImage>, IpcError> {
        if png.len() > MAX_STAMP_PNG_BYTES {
            return Err(IpcError {
                code: ErrorCode::InvalidArgument,
                message: "signature PNG is too large".to_owned(),
            });
        }
        stamp_png_size(png).map_err(|error| IpcError {
            code: ErrorCode::InvalidArgument,
            message: error.to_string(),
        })?;

        let mut inner = self.inner.lock().unwrap();
        let current = inner.list()?;
        if current.len() >= MAX_SIGNATURE_IMAGES {
            return Err(IpcError {
                code: ErrorCode::LimitExceeded,
                message: "too many saved signatures: delete one first".to_owned(),
            });
        }

        let Some(folder) = inner.folder.clone() else {
            return Err(IpcError {
                code: ErrorCode::InvalidArgument,
                message: "signatures folder is not configured".to_owned(),
            });
        };

        inner.refresh_highest_id();
        let next_id = inner.highest_seen_id.checked_add(1).ok_or_else(|| IpcError {
            code: ErrorCode::LimitExceeded,
            message: "signature id overflow".to_owned(),
        })?;
        inner.highest_seen_id = next_id;

        let protected = index_crypto::protect_with(png, ENTROPY).map_err(|_| IpcError {
            code: ErrorCode::Unwritable,
            message: "could not protect signature with DPAPI".to_owned(),
        })?;

        let file_path = folder.join(format!("{next_id}.sig"));
        local_data::write(&file_path, &protected).map_err(|_| IpcError {
            code: ErrorCode::Unwritable,
            message: "could not write signature image file".to_owned(),
        })?;

        inner.list()
    }

    /// Deletes a single signature image file, returning the updated list.
    pub fn remove(&self, id: SignatureImageId) -> Result<Vec<SignatureImage>, IpcError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(folder) = &inner.folder {
            let file_path = folder.join(format!("{}.sig", id.0));
            local_data::remove(&file_path);
        }
        inner.list()
    }

    /// Deletes all signature image files and the directory, returning the updated list (empty).
    pub fn remove_all(&self) -> Result<Vec<SignatureImage>, IpcError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(folder) = &inner.folder {
            if folder.exists() {
                let _ = fs::remove_dir_all(folder);
            }
        }
        inner.list()
    }
}

impl Inner {
    fn refresh_highest_id(&mut self) {
        let Some(folder) = &self.folder else { return };
        if let Ok(entries) = fs::read_dir(folder) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        if let Some(name) = entry.file_name().to_str() {
                            if let Some(id) = parse_signature_file_name(name) {
                                self.highest_seen_id = self.highest_seen_id.max(id);
                            }
                        }
                    }
                }
            }
        }
    }

    fn list(&mut self) -> Result<Vec<SignatureImage>, IpcError> {
        let Some(folder) = &self.folder else {
            return Ok(Vec::new());
        };
        if !folder.exists() {
            return Ok(Vec::new());
        }
        let entries = match fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(_) => return Ok(Vec::new()),
        };

        let mut found: Vec<(u32, u32, u32)> = Vec::new();
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else { continue };
            if !file_type.is_file() { continue };
            let Some(name) = entry.file_name().to_str().map(|s| s.to_owned()) else { continue };
            let Some(id) = parse_signature_file_name(&name) else { continue };
            self.highest_seen_id = self.highest_seen_id.max(id);

            let path = entry.path();
            if let Some((_plain, width, height)) = read_signature_file(&path) {
                found.push((id, width, height));
            }
        }

        found.sort_by_key(|(id, _, _)| *id);
        let signatures = found
            .into_iter()
            .take(MAX_SIGNATURE_IMAGES)
            .map(|(id, width, height)| SignatureImage {
                id: SignatureImageId(id),
                width,
                height,
            })
            .collect();
        Ok(signatures)
    }
}

/// Parses a file name matching `^[0-9]{1,10}\.sig$`.
fn parse_signature_file_name(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".sig")?;
    if stem.is_empty() || stem.len() > 10 || !stem.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    stem.parse::<u32>().ok()
}

/// Reads, decrypts, and validates a signature image file.
fn read_signature_file(path: &Path) -> Option<(Zeroizing<Vec<u8>>, u32, u32)> {
    let bytes = local_data::read(path, MAX_SIGNATURE_FILE_BYTES)?;
    let plain = index_crypto::unprotect_with(&bytes, ENTROPY).ok()?;
    let (width, height) = stamp_png_size(&plain).ok()?;
    Some((plain, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generates a valid minimal RGBA PNG of `width` x `height`.
    fn make_test_png(width: u32, height: u32) -> Vec<u8> {
        let mut raw_scanlines = Vec::new();
        for _y in 0..height {
            raw_scanlines.push(0u8); // filter type none
            for _x in 0..width {
                raw_scanlines.extend_from_slice(&[0, 0, 0, 255]); // RGBA black
            }
        }

        // Deflate uncompressed stream (RFC 1950 zlib)
        let mut zlib = vec![0x78, 0x01];
        let len = raw_scanlines.len();
        assert!(len <= 65535, "test helper bounds check");
        let len_u16 = len as u16;
        let nlen_u16 = !len_u16;
        zlib.push(0x01); // BFINAL=1, BTYPE=00 (uncompressed)
        zlib.extend_from_slice(&len_u16.to_le_bytes());
        zlib.extend_from_slice(&nlen_u16.to_le_bytes());
        zlib.extend_from_slice(&raw_scanlines);

        // Adler32
        let mut s1 = 1u32;
        let mut s2 = 0u32;
        for b in &raw_scanlines {
            s1 = (s1 + *b as u32) % 65521;
            s2 = (s2 + s1) % 65521;
        }
        let adler = (s2 << 16) | s1;
        zlib.extend_from_slice(&adler.to_be_bytes());

        let mut png = Vec::new();
        png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]); // signature

        // IHDR chunk
        let mut ihdr_data = Vec::new();
        ihdr_data.extend_from_slice(&width.to_be_bytes());
        ihdr_data.extend_from_slice(&height.to_be_bytes());
        ihdr_data.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit depth, color type 6 RGBA
        write_chunk(&mut png, b"IHDR", &ihdr_data);

        // IDAT chunk
        write_chunk(&mut png, b"IDAT", &zlib);

        // IEND chunk
        write_chunk(&mut png, b"IEND", &[]);
        png
    }

    fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = Vec::new();
        crc_input.extend_from_slice(kind);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for byte in data {
            crc ^= *byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDBA_7720 & mask);
            }
        }
        !crc
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("test-signatures-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn roundtrip_add_list_get_png() {
        let dir = test_dir("roundtrip");
        let signatures = SignatureImages::new(Some(dir.clone()));

        assert!(signatures.list().unwrap().is_empty());

        let png = make_test_png(20, 10);
        let list = signatures.add(&png).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id.0, 1);
        assert_eq!(list[0].width, 20);
        assert_eq!(list[0].height, 10);

        // Files on disk: not raw PNG
        let sig_path = dir.join("1.sig");
        assert!(sig_path.exists());
        let disk_bytes = fs::read(&sig_path).unwrap();
        assert!(!disk_bytes.starts_with(&[137, 80, 78, 71]));
        assert!(!disk_bytes.windows(png.len()).any(|w| w == png));

        // Text index entropy cannot unprotect it
        assert!(index_crypto::unprotect(&disk_bytes).is_err());

        // Get png
        let retrieved = signatures.get_png(list[0].id).unwrap();
        assert_eq!(retrieved, png);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn limits_and_rejections() {
        let dir = test_dir("limits");
        let signatures = SignatureImages::new(Some(dir.clone()));

        // Invalid PNGs rejected
        assert_eq!(
            signatures.add(b"not a png").unwrap_err().code,
            ErrorCode::InvalidArgument
        );

        // Huge dimensions rejected
        let huge_png = make_test_png(1025, 10);
        assert_eq!(
            signatures.add(&huge_png).unwrap_err().code,
            ErrorCode::InvalidArgument
        );

        // Add 10 valid signatures
        let valid_png = make_test_png(10, 10);
        for i in 1..=10 {
            let list = signatures.add(&valid_png).unwrap();
            assert_eq!(list.len(), i);
        }

        // 11th signature returns LimitExceeded
        assert_eq!(
            signatures.add(&valid_png).unwrap_err().code,
            ErrorCode::LimitExceeded
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_single_and_delete_all() {
        let dir = test_dir("delete");
        let signatures = SignatureImages::new(Some(dir.clone()));

        let png = make_test_png(16, 16);
        let list = signatures.add(&png).unwrap();
        let sig_id = list[0].id;
        assert!(dir.join(format!("{}.sig", sig_id.0)).exists());

        // Remove single
        let after_remove = signatures.remove(sig_id).unwrap();
        assert!(after_remove.is_empty());
        assert!(!dir.join(format!("{}.sig", sig_id.0)).exists());

        // Add two more
        signatures.add(&png).unwrap();
        signatures.add(&png).unwrap();
        assert_eq!(signatures.list().unwrap().len(), 2);
        assert!(dir.exists());

        // Remove all
        let after_all = signatures.remove_all().unwrap();
        assert!(after_all.is_empty());
        assert!(!dir.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_invalid_files_and_empty_when_no_folder() {
        let dir = test_dir("skips");
        fs::create_dir_all(&dir).unwrap();
        let signatures = SignatureImages::new(Some(dir.clone()));

        // Add valid
        let png = make_test_png(12, 12);
        signatures.add(&png).unwrap();

        // Write strange filename
        fs::write(dir.join("abc.sig"), b"bogus").unwrap();
        fs::write(dir.join("12345678901.sig"), b"bogus").unwrap();
        fs::write(dir.join("2.not_sig"), b"bogus").unwrap();
        // Write corrupted content in valid filename
        fs::write(dir.join("99.sig"), b"corrupted bytes").unwrap();
        // Write oversized file
        let oversized = vec![0u8; (MAX_SIGNATURE_FILE_BYTES + 10) as usize];
        fs::write(dir.join("100.sig"), &oversized).unwrap();

        // Listing only contains the 1 valid signature
        let list = signatures.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id.0, 1);

        // When no folder configured
        let no_dir_signatures = SignatureImages::new(None);
        assert!(no_dir_signatures.list().unwrap().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
