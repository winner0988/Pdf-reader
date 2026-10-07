//! Release check (REL-01): starts the given `pdf_worker` executable in the sandbox, opens and
//! renders a generated one-page PDF, and exits 0 only if every step works. CI runs it against
//! the worker the installer put on disk. Not shipped.
//!
//! With `--ocr <scanned.pdf> <tessdata folder>` it also has the worker recognise the text of
//! that scan (B2-10) in English, with the language data from that folder, and checks for the
//! words the corpus sample (`benign/scanned-text.pdf`) is a picture of.
//!
//! Usage: `worker_smoke [path\to\pdf_worker.exe] [--ocr scanned.pdf tessdata]` (default worker:
//! next to this executable).

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

    use std::time::{Duration, Instant};

    use ipc_contract::types::Rotation;
    use ipc_contract::worker::{OcrOutcome, OcrPageState, WorkerRequest, WorkerResponse};
    use worker_host::{HostConfig, WorkerHost, bundled_worker_path};

    pub fn run() -> Result<String, String> {
        let mut args: Vec<_> = std::env::args_os().skip(1).collect();
        let ocr = match args.iter().position(|arg| arg == "--ocr") {
            Some(at) if args.len() >= at + 3 => {
                let mut taken = args.split_off(at);
                let folder = PathBuf::from(taken.pop().expect("the folder"));
                let scan = PathBuf::from(taken.pop().expect("the scan"));
                Some((scan, folder))
            }
            Some(_) => return Err("--ocr needs a scanned PDF and a tessdata folder".to_owned()),
            None => None,
        };
        let worker = match args.into_iter().next() {
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
            let mut summary = format!(
                "worker pid {} opened {} page(s) and rendered {}x{}",
                host.worker_id().unwrap_or_default(),
                document.pages.len(),
                raster.width,
                raster.height
            );
            if let Some((scan, folder)) = &ocr {
                summary.push_str("; ");
                summary.push_str(&recognise(&mut host, scan, folder)?);
            }
            Ok(summary)
        })();
        host.stop();
        std::fs::remove_file(&pdf).ok();
        result
    }

    /// Has the worker read the scan's one page in English and checks the words (B2-10).
    fn recognise(
        host: &mut WorkerHost,
        scan: &std::path::Path,
        folder: &std::path::Path,
    ) -> Result<String, String> {
        let data = std::fs::read(folder.join("eng.traineddata"))
            .map_err(|error| format!("ocr: the language data: {error}"))?;
        let (doc, _) = host
            .open(scan)
            .map_err(|error| format!("ocr: open the scan: {error}"))?;
        let loaded = host
            .request(|request| WorkerRequest::OcrLoad {
                request,
                language: "eng".to_owned(),
                data,
            })
            .map_err(|error| format!("ocr: load the language: {error}"))?;
        if !matches!(loaded, WorkerResponse::OcrLoaded { .. }) {
            return Err(format!("ocr: unexpected response {loaded:?}"));
        }
        let checked = host
            .request(|request| WorkerRequest::OcrPage {
                request,
                doc,
                page_index: 0,
                max_millis: 120_000,
            })
            .map_err(|error| format!("ocr: the page: {error}"))?;
        if !matches!(
            checked,
            WorkerResponse::OcrChecked {
                state: OcrPageState::Queued,
                ..
            }
        ) {
            return Err(format!("ocr: the page was not queued: {checked:?}"));
        }
        let started = Instant::now();
        loop {
            let polled = host
                .request(|request| WorkerRequest::OcrPoll { request })
                .map_err(|error| format!("ocr: poll: {error}"))?;
            if let WorkerResponse::OcrPolled { finished, .. } = polled
                && let Some(page) = finished.first()
            {
                match page.outcome {
                    OcrOutcome::Recognised { .. } => break,
                    other => return Err(format!("ocr: the page came to {other:?}")),
                }
            }
            if started.elapsed() > Duration::from_secs(120) {
                return Err("ocr: the page was not done in two minutes".to_owned());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let text = host
            .request(|request| WorkerRequest::GetPageText {
                request,
                doc,
                page_index: 0,
            })
            .map_err(|error| format!("ocr: the text: {error}"))?;
        let WorkerResponse::PageText { text, .. } = text else {
            return Err(format!("ocr: unexpected response {text:?}"));
        };
        let lines: Vec<&str> = text.lines.iter().map(|line| line.text.as_str()).collect();
        if !(text.recognised && lines.contains(&"PRIVACY FIRST") && lines.contains(&"SECRET PAPER"))
        {
            return Err(format!("ocr: the recognised lines are {lines:?}"));
        }
        Ok(format!("recognised {lines:?} in {:?}", started.elapsed()))
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
