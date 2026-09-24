//! Release check (REL-01): starts the given `pdf_worker` executable in the sandbox, opens and
//! renders a generated one-page PDF, and exits 0 only if every step works. CI runs it against
//! the worker the installer put on disk. Not shipped.
//!
//! Usage: `worker_smoke [path\to\pdf_worker.exe]` (default: next to this executable).

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    match smoke::run() {
        Ok(summary) => {
            println!("ok: {summary}");
            std::process::ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("worker smoke test failed: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod smoke {
    use std::path::PathBuf;

    use ipc_contract::types::Rotation;
    use ipc_contract::worker::{WorkerRequest, WorkerResponse};
    use worker_host::{HostConfig, WorkerHost, bundled_worker_path};

    pub fn run() -> Result<String, String> {
        let worker = match std::env::args_os().nth(1) {
            Some(path) => PathBuf::from(path),
            None => bundled_worker_path().map_err(|error| error.to_string())?,
        };
        if !worker.is_file() {
            return Err(format!("{} does not exist", worker.display()));
        }

        let pdf = std::env::temp_dir().join(format!("worker-smoke-{}.pdf", std::process::id()));
        std::fs::write(&pdf, one_page_pdf()).map_err(|error| error.to_string())?;
        let mut host = WorkerHost::new(&worker, HostConfig::default());
        let result = (|| {
            let (doc, opened) = host.open(&pdf).map_err(|error| format!("open: {error}"))?;
            let WorkerResponse::Opened { document, .. } = opened else {
                return Err(format!("open: unexpected response {opened:?}"));
            };
            let rendered = host
                .request(|request| WorkerRequest::Render {
                    request,
                    doc,
                    page_index: 0,
                    scale: 0.25,
                    rotation: Rotation::None,
                })
                .map_err(|error| format!("render: {error}"))?;
            let WorkerResponse::Rendered { raster, .. } = rendered else {
                return Err(format!("render: unexpected response {rendered:?}"));
            };
            Ok(format!(
                "worker pid {} opened {} page(s) and rendered {}x{}",
                host.worker_id().unwrap_or_default(),
                document.pages.len(),
                raster.width,
                raster.height
            ))
        })();
        host.stop();
        std::fs::remove_file(&pdf).ok();
        result
    }

    /// A Letter page with a black square, with a correct xref table.
    fn one_page_pdf() -> Vec<u8> {
        let content = "0 0 0 rg 100 100 200 200 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_owned(),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        out
    }
}
