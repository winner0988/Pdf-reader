//! Measurement for MVP-07 (docs/architecture/rendering.md): opens a PDF in the sandboxed worker
//! and renders pages the way scrolling does, reporting open time, render times and the worker's
//! peak memory. Not shipped.
//!
//! Usage: `render_bench <file.pdf> [pages=200] [scale=1.5]`
//! (uses the `pdf_worker.exe` next to this executable; build it with the same profile).

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    match bench::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("render_bench: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod bench {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use ipc_contract::raster::fit_scale;
    use ipc_contract::types::Rotation;
    use ipc_contract::worker::{WorkerRequest, WorkerResponse};
    use worker_host::{HostConfig, WorkerHost, bundled_worker_path};

    pub fn run() -> Result<(), String> {
        let mut args = std::env::args().skip(1);
        let pdf = PathBuf::from(
            args.next()
                .ok_or("usage: render_bench <file.pdf> [pages] [scale]")?,
        );
        let pages: usize = args
            .next()
            .map_or(Ok(200), |value| value.parse())
            .map_err(|_| "pages must be a number")?;
        let scale: f32 = args
            .next()
            .map_or(Ok(1.5), |value| value.parse())
            .map_err(|_| "scale must be a number")?;
        let worker = bundled_worker_path().map_err(|error| error.to_string())?;
        let file_bytes = std::fs::metadata(&pdf)
            .map_err(|error| error.to_string())?
            .len();

        let mut host = WorkerHost::new(&worker, HostConfig::default());
        let started = Instant::now();
        let (doc, opened) = host.open(&pdf).map_err(|error| format!("open: {error}"))?;
        let open_time = started.elapsed();
        let WorkerResponse::Opened { document, .. } = opened else {
            return Err("unexpected response to Open".into());
        };

        let mut render = |page_index: usize| -> Result<(Duration, usize), String> {
            let page = document.pages[page_index];
            let scale = fit_scale(page, scale).map_err(|error| error.to_string())?;
            let started = Instant::now();
            let response = host
                .request(|request| WorkerRequest::Render {
                    request,
                    doc,
                    page_index: page_index as u32,
                    scale,
                    rotation: Rotation::None,
                })
                .map_err(|error| format!("render page {}: {error}", page_index + 1))?;
            let WorkerResponse::Rendered { raster, .. } = response else {
                return Err("unexpected response to Render".into());
            };
            Ok((started.elapsed(), raster.pixels.len()))
        };

        let (first_page, first_bytes) = render(0)?;
        let count = pages.min(document.pages.len());
        // Down through the first `count` pages, then back up: like scrolling there and back.
        let order = (1..count).chain((0..count.saturating_sub(1)).rev());
        let mut times = Vec::new();
        for page_index in order {
            times.push(render(page_index)?.0);
        }
        let peak = host.worker_peak_memory().unwrap_or(0);
        times.sort();
        let percentile = |p: f64| {
            times
                .get(((times.len() as f64 - 1.0) * p).round() as usize)
                .copied()
                .unwrap_or_default()
        };
        let total: Duration = times.iter().sum();

        let ms = |duration: Duration| format!("{:.1} ms", duration.as_secs_f64() * 1000.0);
        println!(
            "file              {:.1} MB, {} pages",
            file_bytes as f64 / 1e6,
            document.pages.len()
        );
        println!("open              {}", ms(open_time));
        println!(
            "first page        {} ({:.1} MB raster at scale {scale})",
            ms(first_page),
            first_bytes as f64 / 1e6
        );
        println!("open + first page {}", ms(open_time + first_page));
        println!(
            "{} renders       median {}, p95 {}, max {}, mean {}",
            times.len(),
            ms(percentile(0.5)),
            ms(percentile(0.95)),
            ms(percentile(1.0)),
            ms(total / times.len().max(1) as u32)
        );
        println!("worker peak mem   {:.0} MB", peak as f64 / 1e6);
        Ok(())
    }
}
