//! The real worker binary in the real sandbox, driven by the main-process host (MVP-04).
#![cfg(windows)]
// Two tests inspect the worker process (integrity level, forced termination) through Win32.
#![allow(unsafe_code)]

mod common;

use std::path::Path;
use std::time::Duration;

use ipc_contract::types::{DocumentId, ErrorCode, Rotation};
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use sandbox_config::memory_limited;
use worker_host::{HostConfig, HostError, WorkerHost};

use common::{one_page, temp_pdf};

fn worker() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_pdf_worker"))
}

fn host(config: HostConfig) -> WorkerHost {
    WorkerHost::new(worker(), config)
}

fn square_page() -> Vec<u8> {
    one_page("0 0 0 rg 100 100 200 200 re f")
}

fn render(host: &mut WorkerHost, doc: DocumentId, scale: f32) -> Result<WorkerResponse, HostError> {
    host.request(|request| WorkerRequest::Render {
        request,
        doc,
        page_index: 0,
        scale,
        rotation: Rotation::None,
    })
}

mod sandbox_config {
    use worker_host::HostConfig;

    pub fn memory_limited(megabytes: usize) -> HostConfig {
        let mut config = HostConfig::default();
        config.sandbox.memory_limit_bytes = megabytes * 1024 * 1024;
        config
    }
}

#[test]
fn opens_and_renders_through_the_sandbox() {
    let path = temp_pdf("render", &square_page());
    let mut host = host(HostConfig::default());

    let (doc, opened) = host.open(&path).unwrap();
    let WorkerResponse::Opened { document, .. } = opened else {
        panic!("expected Opened, got {opened:?}");
    };
    assert_eq!(document.pages.len(), 1);
    assert_eq!(
        (document.pages[0].width_pt, document.pages[0].height_pt),
        (612.0, 792.0)
    );

    let WorkerResponse::Rendered { raster, .. } = render(&mut host, doc, 1.0).unwrap() else {
        panic!("expected Rendered");
    };
    assert_eq!((raster.width, raster.height), (612, 792));
    // The black square is at (200, 592) from the top-left.
    let at = ((592 * raster.width + 200) * 4) as usize;
    assert_eq!(&raster.pixels[at..at + 4], &[0, 0, 0, 255]);
    std::fs::remove_file(path).ok();
}

#[test]
fn worker_errors_are_reported_not_fatal() {
    let not_pdf = temp_pdf(
        "not-pdf",
        b"This is a plain text file with a .pdf extension.\n",
    );
    let mut host = host(HostConfig::default());

    let error = host.open(&not_pdf).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotPdf);
    assert!(
        host.is_running(),
        "a bad document must not cost us the worker"
    );

    let error = render(&mut host, DocumentId(999), 1.0).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownDocument);
    std::fs::remove_file(not_pdf).ok();
}

#[test]
fn worker_runs_at_low_integrity_in_an_app_container() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TokenIntegrityLevel, TokenIsAppContainer,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let path = temp_pdf("integrity", &square_page());
    let mut host = host(HostConfig::default());
    host.open(&path).unwrap();
    let pid = host.worker_id().unwrap();

    // SAFETY: plain Win32 queries; every handle is closed and the buffer outlives its use.
    let (rid, is_app_container) = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        assert!(!process.is_null());
        let mut token = std::ptr::null_mut();
        assert_ne!(OpenProcessToken(process, TOKEN_QUERY, &mut token), 0);
        let mut buffer = vec![0u8; 256];
        let mut length = 0u32;
        assert_ne!(
            GetTokenInformation(
                token,
                TokenIntegrityLevel,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut length
            ),
            0
        );
        let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        let rid = *GetSidSubAuthority(label.Label.Sid, u32::from(count) - 1);
        let mut is_app_container = 0u32;
        assert_ne!(
            GetTokenInformation(
                token,
                TokenIsAppContainer,
                (&raw mut is_app_container).cast(),
                size_of::<u32>() as u32,
                &mut length
            ),
            0
        );
        CloseHandle(token);
        CloseHandle(process);
        (rid, is_app_container)
    };
    assert_eq!(rid, 0x1000, "SECURITY_MANDATORY_LOW_RID");
    assert_eq!(is_app_container, 1, "worker must run in an AppContainer");
    std::fs::remove_file(path).ok();
}

#[test]
fn killed_worker_is_reported_then_replaced() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

    let path = temp_pdf("killed", &square_page());
    let mut host = host(HostConfig::default());
    let (doc, _) = host.open(&path).unwrap();
    let first = host.worker_id().unwrap();

    // Simulate a crash from outside, as a memory-safety bug in MuPDF would.
    // SAFETY: plain Win32 calls on the worker's process id; the handle is closed.
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE, 0, first);
        assert!(!process.is_null());
        assert_ne!(TerminateProcess(process, 0xDEAD), 0);
        CloseHandle(process);
    }

    let error = render(&mut host, doc, 1.0).unwrap_err();
    assert_eq!(error.code(), ErrorCode::WorkerCrashed);
    assert!(!host.is_running());

    // The next request starts a fresh worker; documents have to be reopened.
    let (doc, _) = host.open(&path).unwrap();
    assert_ne!(host.worker_id().unwrap(), first);
    assert!(render(&mut host, doc, 1.0).is_ok());
    std::fs::remove_file(path).ok();
}

#[test]
fn slow_requests_time_out_and_the_worker_is_stopped() {
    // Tens of thousands of tiny filled squares take far longer than 50 ms to render.
    let content: String = (0..40_000)
        .map(|i| format!("{} {} 2 2 re f\n", (i % 300) * 2, (i / 300) * 2))
        .collect();
    let path = temp_pdf("slow", &one_page(&content));
    let config = HostConfig {
        request_timeout: Duration::from_millis(50),
        ..HostConfig::default()
    };
    let mut host = host(config);
    let (doc, _) = host.open(&path).unwrap();

    let error = render(&mut host, doc, 4.0).unwrap_err();
    assert_eq!(error.code(), ErrorCode::WorkerTimeout);
    assert!(!host.is_running(), "a worker that timed out is stopped");
    std::fs::remove_file(path).ok();
}

#[test]
fn memory_limit_contains_a_huge_render() {
    let path = temp_pdf("memory", &square_page());
    let mut host = host(memory_limited(96));
    let (doc, _) = host.open(&path).unwrap();

    // 4096 x 4096 is within the raster limit but needs well over 96 MiB here.
    let big = host.request(|request| WorkerRequest::Render {
        request,
        doc,
        page_index: 0,
        scale: 4096.0 / 792.0,
        rotation: Rotation::None,
    });
    assert!(
        big.is_err(),
        "the render must fail inside the worker, not in the main process"
    );

    // Whatever happened in the worker, the host keeps working.
    let (doc, _) = host.open(&path).unwrap();
    assert!(render(&mut host, doc, 0.5).is_ok());
    std::fs::remove_file(path).ok();
}

#[test]
fn worker_binary_imports_no_networking() {
    let imports = pe_imports(&std::fs::read(worker()).unwrap());
    assert!(
        imports.iter().any(|dll| dll == "kernel32.dll"),
        "parser sanity check: {imports:?}"
    );
    for forbidden in [
        "ws2_32.dll",
        "wsock32.dll",
        "mswsock.dll",
        "wininet.dll",
        "winhttp.dll",
        "urlmon.dll",
        "dnsapi.dll",
        "iphlpapi.dll",
        "webio.dll",
        "user32.dll",
        "gdi32.dll",
        "shell32.dll",
    ] {
        assert!(
            !imports.iter().any(|dll| dll == forbidden),
            "pdf_worker must not import {forbidden}: {imports:?}"
        );
    }
}

/// Names (lower-case) of the DLLs in a PE file's import table.
fn pe_imports(image: &[u8]) -> Vec<String> {
    let u16_at = |at: usize| u16::from_le_bytes(image[at..at + 2].try_into().unwrap()) as usize;
    let u32_at = |at: usize| u32::from_le_bytes(image[at..at + 4].try_into().unwrap()) as usize;

    let pe = u32_at(0x3c);
    assert_eq!(&image[pe..pe + 4], b"PE\0\0");
    let sections = u16_at(pe + 6);
    let optional = pe + 24;
    let optional_size = u16_at(pe + 20);
    assert_eq!(u16_at(optional), 0x20b, "PE32+");
    let import_rva = u32_at(optional + 120); // data directory 1 (imports)

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

    let mut names = Vec::new();
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
    names
}
