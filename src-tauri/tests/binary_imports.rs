//! Checks that the main executable (`pdf-reader.exe`) imports no networking DLLs
//! other than `winhttp.dll` (#612).
//!
//! The main process may only reach the network through `winhttp.dll` when the user
//! explicitly checks for updates in `src-tauri/src/update_check.rs` (ADR 0009).
//! Known limitation: this test inspects static and delay-loaded PE import tables only;
//! dynamic loading via `LoadLibraryW` / `GetProcAddress` cannot be detected here and
//! is guarded by `forbidden-patterns.txt` and code review.
#![cfg(windows)]

#[test]
fn main_binary_imports_no_network_dll_except_winhttp() {
    let (imports, delay_imports) =
        pe_imports(&std::fs::read(env!("CARGO_BIN_EXE_pdf-reader")).unwrap());
    assert!(
        imports.iter().any(|dll| dll == "kernel32.dll"),
        "parser sanity check: standard={imports:?}, delay={delay_imports:?}"
    );

    for forbidden in [
        "ws2_32.dll",
        "wsock32.dll",
        "mswsock.dll",
        "wininet.dll",
        "urlmon.dll",
        "dnsapi.dll",
        "iphlpapi.dll",
        "webio.dll",
    ] {
        assert!(
            !imports.iter().any(|dll| dll == forbidden)
                && !delay_imports.iter().any(|dll| dll == forbidden),
            "the main process may only reach the network through winhttp.dll, in update_check.rs (ADR 0009); \
             found forbidden {forbidden}: standard={imports:?}, delay={delay_imports:?}"
        );
    }
}

// Keep in sync with crates/pdf_worker/tests/isolation.rs.
// Do not share via #[path]: every .rs under tests/ becomes its own test target.
/// Names (lower-case) of the DLLs in a PE file's import table:
/// returns `(standard_imports, delay_imports)`.
///
/// Both standard imports (data directory 1) and delay-loaded imports (data directory 13)
/// are inspected because DLLs linked with `/DELAYLOAD` do not appear in the standard
/// import table.
fn pe_imports(image: &[u8]) -> (Vec<String>, Vec<String>) {
    let u16_at = |at: usize| u16::from_le_bytes(image[at..at + 2].try_into().unwrap()) as usize;
    let u32_at = |at: usize| u32::from_le_bytes(image[at..at + 4].try_into().unwrap()) as usize;

    let pe = u32_at(0x3c);
    assert_eq!(&image[pe..pe + 4], b"PE\0\0");
    let sections = u16_at(pe + 6);
    let optional = pe + 24;
    let optional_size = u16_at(pe + 20);
    assert_eq!(u16_at(optional), 0x20b, "PE32+");

    let section_table = optional + optional_size;
    let to_offset = |rva: usize| {
        (0..sections)
            .map(|i| section_table + i * 40)
            .find_map(|s| {
                let (virtual_address, size, raw) = (
                    u32_at(s + 12),
                    u32_at(s + 8).max(u32_at(s + 16)),
                    u32_at(s + 20),
                );
                (rva >= virtual_address && rva < virtual_address + size)
                    .then(|| rva - virtual_address + raw)
            })
            .expect("RVA inside a section")
    };

    // Standard imports: data directory 1 (imports)
    let import_rva = u32_at(optional + 120);
    let mut names = Vec::new();
    if import_rva != 0 {
        let mut descriptor = to_offset(import_rva);
        loop {
            let name_rva = u32_at(descriptor + 12);
            if name_rva == 0 {
                break;
            }
            let start = to_offset(name_rva);
            let end = start + image[start..].iter().position(|&b| b == 0).unwrap();
            names.push(String::from_utf8_lossy(&image[start..end]).to_lowercase());
            descriptor += 20;
        }
    }

    // Delay-loaded imports: data directory 13 (delay import descriptor)
    let num_rva_and_sizes = u32_at(optional + 108);
    let mut delay_names = Vec::new();
    if num_rva_and_sizes >= 14 {
        let delay_rva = u32_at(optional + 112 + 13 * 8);
        if delay_rva != 0 {
            let mut descriptor = to_offset(delay_rva);
            loop {
                let dll_name_rva = u32_at(descriptor + 4);
                if dll_name_rva == 0 {
                    break;
                }
                let attributes = u32_at(descriptor);
                assert!(
                    attributes & 1 != 0,
                    "legacy delay load format is not supported by this test (Attributes bit 0 is 0): {attributes:#x}"
                );
                let start = to_offset(dll_name_rva);
                let end = start + image[start..].iter().position(|&b| b == 0).unwrap();
                delay_names.push(String::from_utf8_lossy(&image[start..end]).to_lowercase());
                descriptor += 32;
            }
        }
    }

    (names, delay_names)
}
