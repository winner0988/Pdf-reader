//! Exporting pages as text or PNG files (B2-04, docs/architecture/export.md). The WebView only
//! says what to export. The main process asks the user where, in the system's dialogs
//! (`file_dialog`), gets each page's text or PNG from the document's worker, and writes the files;
//! no file content passes through the WebView.
//!
//! Pages are fetched as background work on the render thread, as search does, so the view keeps
//! up during a long export.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ipc_contract::types::{DocumentId, ErrorCode, ExportEvent, IpcError, PageText, RequestId};
use tauri::async_runtime::block_on;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::documents::Documents;
use crate::render::Renderer;
use crate::strings;

/// Exports in progress, so `cancel` can stop them.
#[derive(Default)]
pub struct Exports {
    running: Mutex<HashMap<RequestId, Arc<AtomicBool>>>,
}

impl Exports {
    /// Stops the export `request` before its next page. Unknown requests are ignored.
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

    fn lock(&self) -> MutexGuard<'_, HashMap<RequestId, Arc<AtomicBool>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn cancelled() -> IpcError {
    IpcError {
        code: ErrorCode::Cancelled,
        message: "cancelled".to_owned(),
    }
}

fn not_written(error: io::Error) -> IpcError {
    IpcError {
        code: ErrorCode::Unreadable,
        message: format!("the file could not be written ({:?})", error.kind()),
    }
}

/// Writes `file` whole: the content goes to a new temporary file next to it, which then takes its
/// place, so an interrupted export never leaves half a file under the real name.
fn write_whole(file: &Path, content: &[u8]) -> io::Result<()> {
    let (temporary, mut writer) = new_temporary_file(file)?;
    let written = writer.write_all(content);
    drop(writer);
    let written = written.and_then(|()| fs::rename(&temporary, file));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Creates the file to write `file` through: `<file>.tmp`, or `<file>.<n>.tmp` if that name is
/// taken. It is always a new file, so no other file in the user's folder is overwritten.
fn new_temporary_file(file: &Path) -> io::Result<(PathBuf, fs::File)> {
    for attempt in 0..100 {
        let mut name = file.as_os_str().to_owned();
        if attempt > 0 {
            name.push(format!(".{attempt}"));
        }
        name.push(".tmp");
        let temporary = PathBuf::from(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(writer) => return Ok((temporary, writer)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

/// What exported file names start with: the document's file name without `.pdf`.
pub fn stem(display_name: &str) -> String {
    let stem = match display_name.len().checked_sub(4) {
        Some(at)
            if display_name.is_char_boundary(at)
                && display_name[at..].eq_ignore_ascii_case(".pdf") =>
        {
            &display_name[..at]
        }
        _ => display_name,
    };
    if stem.trim().is_empty() {
        "PDF".to_owned()
    } else {
        stem.to_owned()
    }
}

/// Where each exported page image goes: `<stem>-p<page number>.png` in `folder`.
pub fn png_targets(folder: &Path, stem: &str, pages: &[u32]) -> Vec<PathBuf> {
    pages
        .iter()
        .map(|page| folder.join(format!("{stem}-p{}.png", page + 1)))
        .collect()
}

/// The exported text: each page's lines, then a form feed, as `pdftotext` does. A page without
/// text says so instead of being silently empty.
pub fn text_file(pages: &[PageText]) -> String {
    let mut text = String::new();
    for page in pages {
        if page.lines.is_empty() {
            text.push_str(strings::NO_TEXT_LAYER_PAGE);
            text.push_str("\r\n");
        }
        for line in &page.lines {
            text.push_str(&line.text);
            text.push_str("\r\n");
        }
        text.push('\u{c}');
    }
    text
}

/// Runs `work` on each page in turn as background work on the render thread, sending progress,
/// until done or cancelled. Blocks: call it on the blocking pool.
fn each_page<T: Send + 'static>(
    app: &AppHandle,
    request: RequestId,
    pages: &[u32],
    channel: &Channel<ExportEvent>,
    work: impl Fn(&AppHandle, u32) -> Result<T, IpcError> + Send + Sync + Clone + 'static,
    mut done: impl FnMut(u32, T) -> Result<(), IpcError>,
) -> Result<(), IpcError> {
    let exports = app.state::<Exports>();
    let stop = exports.start(request);
    let total = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    let result = (|| {
        for (index, page) in pages.iter().copied().enumerate() {
            if stop.load(Ordering::SeqCst) {
                return Err(cancelled());
            }
            let (handle, work) = (app.clone(), work.clone());
            let value = block_on(
                app.state::<Renderer>()
                    .in_background(move || work(&handle, page)),
            )
            .ok_or_else(cancelled)??;
            done(page, value)?;
            let pages_done = u32::try_from(index + 1).unwrap_or(u32::MAX);
            // Nobody listening any more is no reason to stop writing.
            let _ = channel.send(ExportEvent::Progress { pages_done, total });
        }
        Ok(())
    })();
    exports.finish(request);
    result
}

/// Writes the text of `pages` of `doc` to `file`, whole: nothing is written if it is cancelled.
pub fn write_text(
    app: &AppHandle,
    request: RequestId,
    doc: DocumentId,
    pages: &[u32],
    file: &Path,
    channel: &Channel<ExportEvent>,
) -> Result<(), IpcError> {
    let mut texts = Vec::with_capacity(pages.len());
    each_page(
        app,
        request,
        pages,
        channel,
        move |app, page| app.state::<Documents>().page_text(doc, page),
        |_, text| {
            texts.push(text);
            Ok(())
        },
    )?;
    write_whole(file, text_file(&texts).as_bytes()).map_err(not_written)
}

/// Writes each of `pages` of `doc` as a PNG file at `dpi` to its target. Pages written before a
/// cancel or a failure stay.
pub fn write_pngs(
    app: &AppHandle,
    request: RequestId,
    doc: DocumentId,
    pages: &[u32],
    dpi: u32,
    targets: &[PathBuf],
    channel: &Channel<ExportEvent>,
) -> Result<(), IpcError> {
    let targets: HashMap<u32, &PathBuf> = pages.iter().copied().zip(targets).collect();
    each_page(
        app,
        request,
        pages,
        channel,
        move |app, page| app.state::<Documents>().render_png(doc, page, dpi),
        |page, png| write_whole(targets[&page], &png).map_err(not_written),
    )
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::{Point, Quad, TextLine};

    use super::*;

    #[test]
    fn file_names_come_from_the_document_name() {
        assert_eq!(stem("報告.pdf"), "報告");
        assert_eq!(stem("Report.PDF"), "Report");
        assert_eq!(stem("notes"), "notes");
        assert_eq!(stem(".pdf"), "PDF");
        let targets = png_targets(Path::new(r"C:\out"), "報告", &[0, 9]);
        assert_eq!(
            targets,
            [
                PathBuf::from(r"C:\out\報告-p1.png"),
                PathBuf::from(r"C:\out\報告-p10.png")
            ]
        );
    }

    #[test]
    fn files_are_replaced_whole_and_no_other_file_is_touched() {
        let dir = std::env::temp_dir().join(format!("pdf-reader-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("報告.txt");
        fs::write(&file, "old").unwrap();
        // A file of the user's that happens to have the temporary file's name.
        fs::write(dir.join("報告.txt.tmp"), "the user's").unwrap();

        write_whole(&file, "new".as_bytes()).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(
            fs::read_to_string(dir.join("報告.txt.tmp")).unwrap(),
            "the user's"
        );
        let mut names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, ["報告.txt", "報告.txt.tmp"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_text_file_has_the_lines_of_each_page_and_says_which_have_none() {
        let line = |text: &str| TextLine {
            text: text.to_owned(),
            quad: Quad {
                ul: Point { x: 0.0, y: 0.0 },
                ur: Point { x: 1.0, y: 0.0 },
                ll: Point { x: 0.0, y: 1.0 },
                lr: Point { x: 1.0, y: 1.0 },
            },
            edges: Vec::new(),
        };
        let page = |lines: Vec<TextLine>| PageText {
            lines,
            truncated: false,
        };
        let text = text_file(&[
            page(vec![line("Privacy-first PDF Reader"), line("隱私優先")]),
            page(Vec::new()),
        ]);
        assert_eq!(
            text,
            format!(
                "Privacy-first PDF Reader\r\n隱私優先\r\n\u{c}{}\r\n\u{c}",
                strings::NO_TEXT_LAYER_PAGE
            )
        );
    }
}
