//! Exporting pages as text, PNG or JPEG files (B2-04, #111, docs/architecture/export.md). The
//! WebView only says what to export. The main process asks the user where, in the system's
//! dialogs (`file_dialog`), gets each page's text or image file from the document's worker, and
//! writes the files; no file content passes through the WebView.
//!
//! Pages are fetched as background work on the render thread, as search does, so the view keeps
//! up during a long export.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ipc_contract::types::{
    AnnotationSummary, CompareTextResult, DocumentId, ErrorCode, ExportEvent, IpcError, PageText,
    RequestId,
};
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

/// What exported file names start with: the document's file name without `.pdf`, made into
/// something Windows takes as the start of a file name (#264). The document may come from another
/// system's files (a share, a folder from a Linux machine) and have characters in its name that a
/// Windows file cannot: those become `_`. Windows also drops a space or a period at the end of a
/// name, and reads a device name (`CON`, `NUL`, `COM1`, ...) as the device whatever follows it, an
/// extension too: those get an underscore after the name.
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
    let allowed: String = stem
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let allowed = allowed.trim_end_matches([' ', '.']);
    if allowed.trim().is_empty() {
        return "PDF".to_owned();
    }
    // The device name is what comes before the first period (and the spaces before it).
    let (name, extension) = allowed
        .split_once('.')
        .map_or((allowed, None), |(name, rest)| (name, Some(rest)));
    if is_device_name(name) {
        match extension {
            Some(rest) => format!("{name}_.{rest}"),
            None => format!("{name}_"),
        }
    } else {
        allowed.to_owned()
    }
}

/// Whether Windows takes `name` (without any extension) for a device and not a file.
fn is_device_name(name: &str) -> bool {
    let upper = name.trim_end().to_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
        .is_some_and(|number| {
            let mut chars = number.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('1'..='9' | '¹' | '²' | '³'), None)
            )
        })
}

/// The image file an export writes for each page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// B2-04.
    Png,
    /// #111.
    Jpeg,
}

impl ImageKind {
    fn extension(self) -> &'static str {
        match self {
            ImageKind::Png => "png",
            ImageKind::Jpeg => "jpg",
        }
    }
}

/// Where each exported page image goes: `<stem>-p<page number>.png` (or `.jpg`) in `folder`.
pub fn image_targets(folder: &Path, stem: &str, pages: &[u32], kind: ImageKind) -> Vec<PathBuf> {
    pages
        .iter()
        .map(|page| folder.join(format!("{stem}-p{}.{}", page + 1, kind.extension())))
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

/// A file of a split document (B2-06): where it goes, and which pages of the document it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitFile {
    pub path: PathBuf,
    pub pages: Vec<u32>,
}

/// The files `pages` are split into when each takes `count` of them at most, in `folder`: named
/// by the pages they take, or by their number when those do not follow one another.
pub fn split_targets(folder: &Path, stem: &str, pages: &[u32], count: usize) -> Vec<SplitFile> {
    let chunks: Vec<&[u32]> = pages.chunks(count.max(1)).collect();
    chunks
        .iter()
        .enumerate()
        .map(|(part, chunk)| SplitFile {
            path: folder.join(strings::split_part_name(stem, chunk, part, chunks.len())),
            pages: chunk.to_vec(),
        })
        .collect()
}

/// Writes each of `files` of `doc` in turn, as background work on the render thread, sending
/// progress (in pages) until done or cancelled. Files written before a cancel or a failure stay.
/// Blocks: call it on the blocking pool.
pub fn write_pdfs(
    app: &AppHandle,
    request: RequestId,
    doc: DocumentId,
    files: &[SplitFile],
    channel: &Channel<ExportEvent>,
) -> Result<(), IpcError> {
    let exports = app.state::<Exports>();
    let stop = exports.start(request);
    let total = files.iter().fold(0u32, |sum, file| {
        sum.saturating_add(u32::try_from(file.pages.len()).unwrap_or(u32::MAX))
    });
    let result = (|| {
        let mut pages_done = 0u32;
        for file in files {
            if stop.load(Ordering::SeqCst) {
                return Err(cancelled());
            }
            let (handle, file) = (app.clone(), file.clone());
            block_on(app.state::<Renderer>().in_background(move || {
                handle
                    .state::<Documents>()
                    .save_pages(doc, &file.pages, &file.path)
                    .map(|()| file.pages.len())
            }))
            .ok_or_else(cancelled)?
            .map(|written| {
                pages_done = pages_done.saturating_add(u32::try_from(written).unwrap_or(u32::MAX));
            })?;
            // Nobody listening any more is no reason to stop writing.
            let _ = channel.send(ExportEvent::Progress { pages_done, total });
        }
        Ok(())
    })();
    exports.finish(request);
    result
}

/// Writes `pages` of `doc` to `file` as a Word document (B3-02): one request to the document's
/// worker, as background work on the render thread, with progress at the start and the end (the
/// worker says nothing between). Nothing is written if it is cancelled before it starts, and what
/// the worker is making cannot be stopped: the file is replaced only when it is done.
/// Blocks: call it on the blocking pool.
pub fn write_docx(
    app: &AppHandle,
    request: RequestId,
    doc: DocumentId,
    pages: &[u32],
    file: &Path,
    channel: &Channel<ExportEvent>,
) -> Result<(), IpcError> {
    let exports = app.state::<Exports>();
    let stop = exports.start(request);
    let total = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    let result = (|| {
        if stop.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        let _ = channel.send(ExportEvent::Progress {
            pages_done: 0,
            total,
        });
        let (handle, pages, file) = (app.clone(), pages.to_vec(), file.to_path_buf());
        block_on(
            app.state::<Renderer>()
                .in_background(move || handle.state::<Documents>().docx_export(doc, &pages, &file)),
        )
        .ok_or_else(cancelled)??;
        let _ = channel.send(ExportEvent::Progress {
            pages_done: total,
            total,
        });
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

/// A summary of annotations (B3-18) as a file: plain text with the page and kind on each line,
/// or Markdown with a heading for each page and a list item for each annotation; first the
/// document's `name`, if given, and last a note if the summary stopped before the end. A drawing,
/// a stamp, or a mark that covers no text is listed by its kind alone. Lines end in `\n`.
pub fn annotation_summary_file(
    summary: &AnnotationSummary,
    markdown: bool,
    name: Option<&str>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(name) = name {
        lines.push(if markdown {
            format!("# {}", escape_markdown(name))
        } else {
            name.to_owned()
        });
    }
    let mut page = None;
    for item in &summary.items {
        if page != Some(item.page_index) {
            page = Some(item.page_index);
            if !lines.is_empty() {
                lines.push(String::new());
            }
            if markdown {
                lines.push(format!(
                    "## {}",
                    strings::annotation_summary_page(item.page_index + 1)
                ));
                lines.push(String::new());
            }
        }
        let kind = strings::annotation_kind_name(item.kind);
        let mut text = item.text.split('\n');
        let first = text.next().filter(|line| !line.is_empty());
        if markdown {
            let mut entry = match first {
                Some(first) => format!("- **{kind}**：{}", escape_markdown(first)),
                None => format!("- **{kind}**"),
            };
            // The lines of a note stay lines (a hard break), in the same list item; an empty
            // one would end the item.
            for line in text.filter(|line| !line.is_empty()) {
                entry.push_str("\\\n  ");
                entry.push_str(&escape_markdown(line));
            }
            lines.push(entry);
        } else {
            let page = strings::annotation_summary_page(item.page_index + 1);
            lines.push(match first {
                Some(first) => format!("{page}｜{kind}：{first}"),
                None => format!("{page}｜{kind}"),
            });
            lines.extend(text.map(|line| {
                if line.is_empty() {
                    String::new()
                } else {
                    format!("  {line}")
                }
            }));
        }
    }
    if summary.truncated {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(strings::ANNOTATION_SUMMARY_TRUNCATED.to_owned());
    }
    let mut file = lines.join("\n");
    file.push('\n');
    file
}

/// `text` (one line) as Markdown that reads as the text itself: each character Markdown gives a
/// meaning to is escaped, and so is what would start a list, a heading or a quote at the start
/// of a line.
fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '~' | '#' | '&'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    let digits = out.chars().take_while(char::is_ascii_digit).count();
    if out.starts_with(['-', '+', '=']) {
        out.insert(0, '\\');
    } else if digits > 0 && out[digits..].starts_with(['.', ')']) {
        out.insert(digits, '\\');
    }
    out
}

/// Writes the annotation summary `content` (B3-18) to `file`, whole, with Windows line ends as
/// the text export has.
pub fn write_annotation_summary(file: &Path, content: &str) -> Result<(), IpcError> {
    let content = content.replace("\r\n", "\n").replace('\n', "\r\n");
    write_whole(file, content.as_bytes()).map_err(not_written)
}

/// Formats a text comparison outcome as Plain text or Markdown (B3-24, E9).
pub fn compare_text_file(
    result: &CompareTextResult,
    markdown: bool,
    name_a: &str,
    name_b: &str,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    let title = format!("{name_a} 與 {name_b}（文字比較）");

    if markdown {
        lines.push(format!("# {}", escape_markdown(&title)));
        lines.push(String::new());
    } else {
        lines.push(title);
        lines.push(String::new());
    }

    if result.identical {
        lines.push(strings::COMPARE_IDENTICAL.to_owned());
    } else if result.empty {
        lines.push(strings::COMPARE_EMPTY.to_owned());
    } else {
        for (i, item) in result.items.iter().enumerate() {
            let page_a = item.page_a + 1;
            let page_b = item.page_b + 1;
            let header = format!("第 {page_a} 頁（{name_a}）/ 第 {page_b} 頁（{name_b}）");

            if markdown {
                lines.push(format!("## {}. {}", i + 1, escape_markdown(&header)));
                lines.push(String::new());
                if !item.deleted.is_empty() {
                    let del = escape_markdown(&item.deleted);
                    lines.push(format!("- **刪除**：~~{del}~~"));
                }
                if !item.inserted.is_empty() {
                    let ins = escape_markdown(&item.inserted);
                    lines.push(format!("- **新增**：{ins}"));
                }
                lines.push(String::new());
            } else {
                lines.push(format!("{}. {header}", i + 1));
                if !item.deleted.is_empty() {
                    lines.push(format!("  - 刪除：{}", item.deleted));
                }
                if !item.inserted.is_empty() {
                    lines.push(format!("  + 新增：{}", item.inserted));
                }
                lines.push(String::new());
            }
        }
    }

    if result.truncated {
        lines.push(strings::COMPARE_TRUNCATED.to_owned());
    }
    if result.recognised {
        lines.push(strings::COMPARE_RECOGNISED_NOTE.to_owned());
    }

    lines.join("\n")
}

/// Writes the text comparison `content` (B3-24, E9) to `file`, whole, with Windows line ends.
pub fn write_compare_text(file: &Path, content: &str) -> Result<(), IpcError> {
    let content = content.replace("\r\n", "\n").replace('\n', "\r\n");
    write_whole(file, content.as_bytes()).map_err(not_written)
}


/// Writes each of `pages` of `doc` as an image file of `kind` at `dpi` to its target. Pages
/// written before a cancel or a failure stay.
#[allow(clippy::too_many_arguments)]
pub fn write_images(
    app: &AppHandle,
    request: RequestId,
    doc: DocumentId,
    pages: &[u32],
    dpi: u32,
    kind: ImageKind,
    targets: &[PathBuf],
    channel: &Channel<ExportEvent>,
) -> Result<(), IpcError> {
    let targets: HashMap<u32, &PathBuf> = pages.iter().copied().zip(targets).collect();
    each_page(
        app,
        request,
        pages,
        channel,
        move |app, page| app.state::<Documents>().render_image(doc, page, dpi, kind),
        |page, image| write_whole(targets[&page], &image).map_err(not_written),
    )
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::{
        AnnotationSummaryItem, AnnotationSummaryKind, Point, Quad, Rect, TextLine,
    };

    use super::*;

    #[test]
    fn file_names_come_from_the_document_name() {
        assert_eq!(stem("報告.pdf"), "報告");
        assert_eq!(stem("Report.PDF"), "Report");
        assert_eq!(stem("notes"), "notes");
        assert_eq!(stem(".pdf"), "PDF");
        let targets = image_targets(Path::new(r"C:\out"), "報告", &[0, 9], ImageKind::Png);
        assert_eq!(
            targets,
            [
                PathBuf::from(r"C:\out\報告-p1.png"),
                PathBuf::from(r"C:\out\報告-p10.png")
            ]
        );
        let targets = image_targets(Path::new(r"C:\out"), "報告", &[2], ImageKind::Jpeg);
        assert_eq!(targets, [PathBuf::from(r"C:\out\報告-p3.jpg")]);
    }

    #[test]
    fn what_windows_does_not_allow_in_a_file_name_is_not_in_the_start_of_one() {
        // A document from another system's files (#264).
        assert_eq!(stem("report?final*<1>.pdf"), "report_final__1_");
        assert_eq!(stem("a:b|c\"d/e\\f.pdf"), "a_b_c_d_e_f");
        assert_eq!(stem("tab\there\u{1}.pdf"), "tab_here_");
        assert_eq!(
            stem("報告？.pdf"),
            "報告？",
            "a full-width mark is a different character"
        );
        // A space or a period at the end is dropped by Windows: it never reaches the name.
        assert_eq!(stem("name . .pdf"), "name");
        assert_eq!(stem("name...pdf"), "name");
        assert_eq!(stem("...pdf"), "PDF");
        assert_eq!(stem("???.pdf"), "___");
        assert_eq!(stem("   .pdf"), "PDF");
        // Devices, with or without an extension, in any case.
        for (name, expected) in [
            ("CON.pdf", "CON_"),
            ("nul", "nul_"),
            ("Prn .pdf", "Prn_"),
            ("Com1.pdf", "Com1_"),
            ("LPT9.old.pdf", "LPT9_.old"),
            ("AUX.txt.pdf", "AUX_.txt"),
            ("conin$.pdf", "conin$_"),
            ("COM\u{b2}.pdf", "COM\u{b2}_"),
        ] {
            assert_eq!(stem(name), expected, "{name}");
        }
        // Not devices.
        for name in [
            "COM10.pdf",
            "COM0.pdf",
            "console.pdf",
            "LPT.pdf",
            "xCON.pdf",
            "a.con.pdf",
        ] {
            assert_eq!(stem(name), name.trim_end_matches(".pdf"), "{name}");
        }
        // The names built from it are names Windows takes.
        let bad = stem("a?b*c.pdf");
        let [target] = &image_targets(Path::new(r"C:\out"), &bad, &[0], ImageKind::Png)[..] else {
            panic!("one target")
        };
        assert_eq!(target, &PathBuf::from(r"C:\out\a_b_c-p1.png"));
    }

    #[test]
    fn a_split_makes_files_of_so_many_pages_named_by_them() {
        let pages: Vec<u32> = (0..10).collect();
        let files = split_targets(Path::new(r"C:\out"), "報告", &pages, 4);
        let named: Vec<(PathBuf, Vec<u32>)> = files
            .into_iter()
            .map(|file| (file.path, file.pages))
            .collect();
        assert_eq!(
            named,
            [
                (PathBuf::from(r"C:\out\報告-p1-4.pdf"), vec![0, 1, 2, 3]),
                (PathBuf::from(r"C:\out\報告-p5-8.pdf"), vec![4, 5, 6, 7]),
                (PathBuf::from(r"C:\out\報告-p9-10.pdf"), vec![8, 9]),
            ]
        );
        // One page a file; a choice that skips pages is numbered, not named by them.
        let each = split_targets(Path::new(r"C:\out"), "報告", &[0, 1], 1);
        assert_eq!(each[1].path, PathBuf::from(r"C:\out\報告-p2.pdf"));
        let skipping = split_targets(Path::new(r"C:\out"), "報告", &[0, 2, 4, 6], 2);
        assert_eq!(skipping[0].path, PathBuf::from(r"C:\out\報告-1.pdf"));
        assert_eq!(skipping[1].pages, [4, 6]);
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
    fn an_annotation_summary_lists_each_annotation_with_its_page_and_kind() {
        let item =
            |page_index: u32, kind: AnnotationSummaryKind, text: &str| AnnotationSummaryItem {
                page_index,
                kind,
                rect: Rect {
                    x0: 72.0,
                    y0: 72.0,
                    x1: 200.0,
                    y1: 90.0,
                },
                text: text.to_owned(),
            };
        let summary = AnnotationSummary {
            items: vec![
                item(0, AnnotationSummaryKind::Note, "第一行\n\n- second *line*"),
                item(
                    0,
                    AnnotationSummaryKind::Highlight,
                    "Privacy-first PDF Reader",
                ),
                item(0, AnnotationSummaryKind::Highlight, ""),
                item(
                    2,
                    AnnotationSummaryKind::TextBox,
                    "1. <b>#1</b> [x](y) a_b \\ c|d",
                ),
                item(2, AnnotationSummaryKind::Ink, ""),
                item(2, AnnotationSummaryKind::Stamp, ""),
            ],
            truncated: false,
        };
        assert_eq!(
            annotation_summary_file(&summary, false, Some("報告.pdf")),
            "報告.pdf\n\
             \n\
             第 1 頁｜附註：第一行\n\
             \n\
             \x20 - second *line*\n\
             第 1 頁｜螢光筆標示：Privacy-first PDF Reader\n\
             第 1 頁｜螢光筆標示\n\
             \n\
             第 3 頁｜文字方塊：1. <b>#1</b> [x](y) a_b \\ c|d\n\
             第 3 頁｜手繪線條\n\
             第 3 頁｜印章\n"
        );
        assert_eq!(
            annotation_summary_file(&summary, true, Some("報告_v2.pdf")),
            "# 報告\\_v2.pdf\n\
             \n\
             ## 第 1 頁\n\
             \n\
             - **附註**：第一行\\\n\
             \x20 \\- second \\*line\\*\n\
             - **螢光筆標示**：Privacy-first PDF Reader\n\
             - **螢光筆標示**\n\
             \n\
             ## 第 3 頁\n\
             \n\
             - **文字方塊**：1\\. \\<b\\>\\#1\\</b\\> \\[x\\](y) a\\_b \\\\ c\\|d\n\
             - **手繪線條**\n\
             - **印章**\n"
        );
        // Without the name, and a summary that stopped before the end says so.
        let cut = AnnotationSummary {
            items: vec![item(4, AnnotationSummaryKind::Ink, "")],
            truncated: true,
        };
        assert_eq!(
            annotation_summary_file(&cut, false, None),
            format!(
                "第 5 頁｜手繪線條\n\n{}\n",
                strings::ANNOTATION_SUMMARY_TRUNCATED
            )
        );
        assert_eq!(
            annotation_summary_file(&cut, true, None),
            format!(
                "## 第 5 頁\n\n- **手繪線條**\n\n{}\n",
                strings::ANNOTATION_SUMMARY_TRUNCATED
            )
        );
    }

    #[test]
    fn markdown_escapes_leave_ordinary_text_alone() {
        assert_eq!(
            escape_markdown("隱私優先的 PDF 閱讀器"),
            "隱私優先的 PDF 閱讀器"
        );
        assert_eq!(escape_markdown("+1 = 2"), "\\+1 = 2");
        assert_eq!(escape_markdown("=== title"), "\\=== title");
        assert_eq!(escape_markdown("2024) done"), "2024\\) done");
        assert_eq!(escape_markdown("2024 done."), "2024 done.");
        assert_eq!(escape_markdown("> quote"), "\\> quote");
        assert_eq!(escape_markdown("&amp;"), "\\&amp;");
    }

    #[test]
    fn an_annotation_summary_is_written_whole_with_windows_line_ends() {
        let dir = std::env::temp_dir().join(format!("pdf-reader-summary-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("報告（註解摘要）.md");
        write_annotation_summary(&file, "# 報告\n\n- **螢光筆**：重點\r\n").unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "# 報告\r\n\r\n- **螢光筆**：重點\r\n"
        );
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["報告（註解摘要）.md"]);
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
            recognised: false,
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

    #[test]
    fn compare_text_file_formatting_and_escapes() {
        use ipc_contract::types::TextDiffItem;

        let result = CompareTextResult {
            items: vec![
                TextDiffItem {
                    page_a: 0,
                    page_b: 0,
                    deleted: "Party *Beta*".to_string(),
                    inserted: "Party _Gamma_".to_string(),
                    truncated: false,
                },
                TextDiffItem {
                    page_a: 1,
                    page_b: 2,
                    deleted: "$100".to_string(),
                    inserted: "$200".to_string(),
                    truncated: false,
                },
            ],
            identical: false,
            empty: false,
            recognised: true,
            page_text_truncated: false,
            truncated: false,
        };

        let txt = compare_text_file(&result, false, "v1.pdf", "v2.pdf");
        assert!(txt.contains("v1.pdf 與 v2.pdf（文字比較）"));
        assert!(txt.contains("1. 第 1 頁（v1.pdf）/ 第 1 頁（v2.pdf）"));
        assert!(txt.contains("  - 刪除：Party *Beta*"));
        assert!(txt.contains("  + 新增：Party _Gamma_"));
        assert!(txt.contains("2. 第 2 頁（v1.pdf）/ 第 3 頁（v2.pdf）"));
        assert!(txt.contains(strings::COMPARE_RECOGNISED_NOTE));

        let md = compare_text_file(&result, true, "v1.pdf", "v2.pdf");
        assert!(md.contains("# v1.pdf 與 v2.pdf（文字比較）"));
        assert!(md.contains("## 1. 第 1 頁（v1.pdf）/ 第 1 頁（v2.pdf）"));
        assert!(md.contains("- **刪除**：~~Party \\*Beta\\*~~"));
        assert!(md.contains("- **新增**：Party \\_Gamma\\_"));
        assert!(md.contains(strings::COMPARE_RECOGNISED_NOTE));
    }
}
