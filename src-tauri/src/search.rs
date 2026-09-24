//! Full-text search of the open document (MVP-10).
//!
//! The main process walks the pages and asks the worker to search one page at a time, as
//! background work on the render thread: renders still come first, so the view keeps up while a
//! long search runs. Results stream to the frontend over a channel. A search stops when it is
//! cancelled (a new search or closing the search bar cancels the old one), when the document
//! closes, or at `MAX_SEARCH_HITS` hits.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ipc_contract::limits::MAX_SEARCH_HITS;
use ipc_contract::types::{ErrorCode, IpcError, RequestId, SearchArgs, SearchEvent};
use ipc_contract::validate::Validate;
use tauri::async_runtime::block_on;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::documents::{Documents, PageFound};
use crate::render::Renderer;

/// How often progress is reported while searching.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Searches in progress, so `cancel` can stop them.
#[derive(Default)]
pub struct Searches {
    running: Mutex<HashMap<RequestId, Arc<AtomicBool>>>,
}

impl Searches {
    /// Stops the search `request` before its next page. Unknown requests are ignored.
    pub fn cancel(&self, request: RequestId) {
        if let Some(flag) = self.lock().get(&request) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn start(&self, request: RequestId) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.lock().insert(request, flag.clone());
        flag
    }

    fn finish(&self, request: RequestId) {
        self.lock().remove(&request);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RequestId, Arc<AtomicBool>>> {
        self.running
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

fn cancelled() -> IpcError {
    IpcError {
        code: ErrorCode::Cancelled,
        message: "cancelled".to_owned(),
    }
}

/// Where a search stands.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub total_hits: u32,
    pub truncated: bool,
    pub has_text: bool,
}

/// Runs the search described by `args`, sending hits, progress and a final `Done` on
/// `channel`. Returns an error if the search could not complete (cancelled, document closed,
/// worker failure); the frontend then ignores anything it received for that request.
/// Blocks until the search ends: call it on the blocking pool.
pub fn run(
    app: &AppHandle,
    args: SearchArgs,
    channel: Channel<SearchEvent>,
) -> Result<(), IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let page_count = app
        .state::<Documents>()
        .page_count(args.doc)
        .ok_or_else(|| IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })?;
    let searches = app.state::<Searches>();
    let stop = searches.start(args.request);
    let search_page = |page_index: u32, remaining: u32| {
        let (handle, query, doc, case_sensitive) = (
            app.clone(),
            args.query.clone(),
            args.doc,
            args.case_sensitive,
        );
        block_on(app.state::<Renderer>().in_background(move || {
            handle.state::<Documents>().search_page(
                doc,
                page_index,
                &query,
                case_sensitive,
                remaining,
            )
        }))
        .ok_or_else(cancelled)?
    };
    let result = walk(page_count, &stop, search_page, |event| {
        channel.send(event).is_ok()
    });
    searches.finish(args.request);
    let tally = result?;
    let _ = channel.send(SearchEvent::Done {
        total_hits: tally.total_hits,
        truncated: tally.truncated,
        no_text_layer: !tally.has_text,
    });
    Ok(())
}

/// Searches pages `0..page_count` in order with `search_page(page, hits still allowed)`,
/// sending hits and progress through `send` (false: nobody is listening). Stops early when
/// `stop` is set, when `send` fails or at `MAX_SEARCH_HITS`.
pub fn walk(
    page_count: u32,
    stop: &AtomicBool,
    mut search_page: impl FnMut(u32, u32) -> Result<PageFound, IpcError>,
    mut send: impl FnMut(SearchEvent) -> bool,
) -> Result<Tally, IpcError> {
    let mut tally = Tally::default();
    let mut last_progress = Instant::now();
    for page_index in 0..page_count {
        if stop.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        let found = search_page(page_index, MAX_SEARCH_HITS - tally.total_hits)?;
        if stop.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        tally.has_text |= found.has_text;
        if !found.hits.is_empty() {
            let count = u32::try_from(found.hits.len()).unwrap_or(MAX_SEARCH_HITS);
            tally.total_hits = (tally.total_hits + count).min(MAX_SEARCH_HITS);
            if !send(SearchEvent::Hits {
                page_index,
                hits: found.hits,
            }) {
                return Err(cancelled()); // nobody is listening any more
            }
        }
        let done = page_index + 1 == page_count;
        if tally.total_hits >= MAX_SEARCH_HITS {
            // The page itself may have had more hits than were allowed.
            tally.truncated = true;
            send(SearchEvent::Progress {
                pages_searched: page_index + 1,
            });
            break;
        }
        if done || last_progress.elapsed() >= PROGRESS_INTERVAL {
            last_progress = Instant::now();
            send(SearchEvent::Progress {
                pages_searched: page_index + 1,
            });
        }
    }
    Ok(tally)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use ipc_contract::types::{Point, Quad, SearchHit};

    use super::*;

    fn hits(count: usize) -> Vec<SearchHit> {
        let point = Point { x: 1.0, y: 2.0 };
        let quad = Quad {
            ul: point,
            ur: point,
            ll: point,
            lr: point,
        };
        vec![SearchHit { quads: vec![quad] }; count]
    }

    type Outcome = (Result<Tally, IpcError>, Vec<SearchEvent>, Vec<u32>);

    /// Runs `walk` over `pages`, where `pages[i]` is (hit count, has text) of page i.
    fn search(pages: &[(usize, bool)], stop_after: Option<u32>) -> Outcome {
        let stop = AtomicBool::new(false);
        let events = RefCell::new(Vec::new());
        let asked = RefCell::new(Vec::new());
        let result = walk(
            pages.len() as u32,
            &stop,
            |page, remaining| {
                asked.borrow_mut().push(page);
                if stop_after == Some(page) {
                    stop.store(true, Ordering::SeqCst);
                }
                let (count, has_text) = pages[page as usize];
                Ok(PageFound {
                    hits: hits(count.min(remaining as usize)),
                    has_text,
                })
            },
            |event| {
                events.borrow_mut().push(event);
                true
            },
        );
        (result, events.into_inner(), asked.into_inner())
    }

    #[test]
    fn streams_hits_page_by_page_then_finishes() {
        let (result, events, asked) = search(&[(0, true), (2, true), (0, false), (1, true)], None);
        assert_eq!(
            result.unwrap(),
            Tally {
                total_hits: 3,
                truncated: false,
                has_text: true
            }
        );
        assert_eq!(asked, [0, 1, 2, 3]);
        let hit_pages: Vec<u32> = events
            .iter()
            .filter_map(|event| match event {
                SearchEvent::Hits { page_index, .. } => Some(*page_index),
                _ => None,
            })
            .collect();
        assert_eq!(hit_pages, [1, 3]);
        assert!(matches!(
            events.last(),
            Some(SearchEvent::Progress { pages_searched: 4 })
        ));
    }

    #[test]
    fn a_document_without_text_is_reported() {
        let (result, _, _) = search(&[(0, false), (0, false)], None);
        assert!(!result.unwrap().has_text);
    }

    #[test]
    fn stops_at_the_hit_limit() {
        let limit = MAX_SEARCH_HITS as usize;
        let (result, events, asked) = search(&[(limit - 10, true), (50, true), (5, true)], None);
        let tally = result.unwrap();
        assert_eq!(tally.total_hits, MAX_SEARCH_HITS);
        assert!(tally.truncated);
        assert_eq!(asked, [0, 1], "no page is searched after the limit");
        let sent: usize = events
            .iter()
            .map(|event| match event {
                SearchEvent::Hits { hits, .. } => hits.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(sent, limit);
    }

    #[test]
    fn cancelling_stops_before_the_next_page() {
        let (result, events, asked) =
            search(&[(1, true), (1, true), (1, true), (1, true)], Some(1));
        assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
        assert_eq!(asked, [0, 1]);
        // Page 1's hits arrived after the cancel and are dropped.
        let sent = events
            .iter()
            .filter(|event| matches!(event, SearchEvent::Hits { .. }))
            .count();
        assert_eq!(sent, 1);
    }

    #[test]
    fn errors_end_the_search() {
        let stop = AtomicBool::new(false);
        let result = walk(
            3,
            &stop,
            |page, _| {
                if page == 1 {
                    Err(IpcError {
                        code: ErrorCode::UnknownDocument,
                        message: String::new(),
                    })
                } else {
                    Ok(PageFound {
                        hits: Vec::new(),
                        has_text: true,
                    })
                }
            },
            |_| true,
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::UnknownDocument);
    }

    #[test]
    fn a_frontend_that_went_away_stops_the_search() {
        let stop = AtomicBool::new(false);
        let result = walk(
            3,
            &stop,
            |_, _| {
                Ok(PageFound {
                    hits: hits(1),
                    has_text: true,
                })
            },
            |_| false,
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    }

    #[test]
    fn cancel_marks_only_running_searches() {
        let searches = Searches::default();
        let flag = searches.start(RequestId(4));
        searches.cancel(RequestId(5));
        assert!(!flag.load(Ordering::SeqCst));
        searches.cancel(RequestId(4));
        assert!(flag.load(Ordering::SeqCst));
        searches.finish(RequestId(4));
        searches.cancel(RequestId(4));
    }
}
