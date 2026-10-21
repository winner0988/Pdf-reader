//! Measurement for MVP-10 (docs/architecture/search.md): opens a PDF in the sandboxed worker and
//! searches it page by page the way the main process does, reporting the time to the first hit,
//! to page 500 and to the end. Not shipped.
//!
//! Usage: `search_bench <file.pdf> <query> [--case-sensitive]`
//! (uses the `pdf_worker.exe` next to this executable; build it with the same profile).

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    match bench::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("search_bench: {message}");
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

    use ipc_contract::limits::MAX_SEARCH_HITS;
    use ipc_contract::worker::{WorkerRequest, WorkerResponse};
    use worker_host::{HostConfig, WorkerHost, bundled_worker_path};

    pub fn run() -> Result<(), String> {
        let mut args = std::env::args().skip(1);
        let usage = "usage: search_bench <file.pdf> <query> [--case-sensitive]";
        let pdf = PathBuf::from(args.next().ok_or(usage)?);
        let query = args.next().ok_or(usage)?;
        let case_sensitive = args.next().as_deref() == Some("--case-sensitive");
        let worker = bundled_worker_path().map_err(|error| error.to_string())?;

        let mut host = WorkerHost::new(&worker, HostConfig::default());
        let (doc, opened) = host.open(&pdf).map_err(|error| format!("open: {error}"))?;
        let WorkerResponse::Opened { document, .. } = opened else {
            return Err("unexpected response to Open".into());
        };
        let pages = document.pages.len() as u32;

        let ms = |duration: Option<Duration>| {
            duration.map_or("-".to_owned(), |d| {
                format!("{:.0} ms", d.as_secs_f64() * 1000.0)
            })
        };

        println!("{pages} pages, query {query:?}, case sensitive: {case_sensitive}");

        let run_search = |host: &mut WorkerHost| -> Result<_, String> {
            let started = Instant::now();
            let (mut hits, mut first_hit, mut at_500) = (0u32, None::<Duration>, None::<Duration>);
            for page_index in 0..pages {
                let response = host
                    .request(|request| WorkerRequest::SearchPage {
                        request,
                        doc,
                        page_index,
                        query: query.clone(),
                        case_sensitive,
                        max_hits: MAX_SEARCH_HITS - hits,
                    })
                    .map_err(|error| format!("page {}: {error}", page_index + 1))?;
                let WorkerResponse::PageSearched { hits: found, .. } = response else {
                    return Err("unexpected response to SearchPage".into());
                };
                if !found.is_empty() && first_hit.is_none() {
                    first_hit = Some(started.elapsed());
                }
                hits += found.len() as u32;
                if page_index + 1 == 500 {
                    at_500 = Some(started.elapsed());
                }
                if hits >= MAX_SEARCH_HITS {
                    break;
                }
            }
            Ok((hits, first_hit, at_500, started.elapsed()))
        };

        let (hits1, first1, at_500_1, total1) = run_search(&mut host)?;
        println!("first search (cold):");
        println!("  hits            {hits1}");
        println!("  first hit       {}", ms(first1));
        println!("  first 500 pages {}", ms(at_500_1));
        println!("  all pages       {}", ms(Some(total1)));

        let (hits2, first2, at_500_2, total2) = run_search(&mut host)?;
        println!("second search (warm):");
        println!("  hits            {hits2}");
        println!("  first hit       {}", ms(first2));
        println!("  first 500 pages {}", ms(at_500_2));
        println!("  all pages       {}", ms(Some(total2)));
        Ok(())
    }
}
