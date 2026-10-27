//! Batches go on with the window closed (B3-04, ADR 0005, docs/architecture/batch.md; what
//! CONTEXT.md calls the batch processing background service).
//!
//! When the user closes the window while a batch is running (after being asked about unsaved
//! changes, as for any close), the window is hidden instead, and an icon in the notification area
//! says how far the batch has come. Its menu shows the window again, which calls the close off
//! (everything is as it was), or stops the batch. When the batch ends with the window hidden, a
//! report of each file goes into the batch's folder, a message says how it went, and the app
//! closes as it would have. There is no icon and nothing in the background without a batch the
//! user started, and nothing starts with Windows.
//!
//! What the window and the icon do is decided on the main thread, one thing after another: a
//! batch only tells it how far it has come and that it has ended, and the window is only hidden
//! while the main thread knows a batch is running.

use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ipc_contract::types::{BatchEvent, BatchOutcome, BatchReport};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::batch::{self, Batches};
use crate::documents::Documents;
use crate::strings;

/// The notification area icon's id, and its menu's items'.
const ICON: &str = "batch";
const SHOW: &str = "batch-show";
const STOP: &str = "batch-stop";

/// The app's one window.
const WINDOW: &str = "main";

/// Most reports of the same name in one folder: `批次處理報告 (2).txt` and so on.
const MAX_REPORT_NUMBER: u32 = 999;

/// Whether the window was closed while a batch ran, and how far the batches have come.
#[derive(Default)]
pub struct Background {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    mode: Mode,
    /// Files done, and files, of the batch that started last.
    progress: (u32, u32),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    /// The window is shown (or closing as any window does).
    #[default]
    Open,
    /// The user closed the window while a batch ran: it is hidden, and the icon is shown.
    /// `discard`: they chose to discard unsaved changes when they closed it.
    Hidden { discard: bool },
    /// The batch ended with the window hidden: its report is written, and the message says so;
    /// the app closes once the user has read it.
    Closing { discard: bool },
}

impl Background {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The user closed the window, and was asked about unsaved changes (`discard` is what they
/// chose): while a batch runs, the window is hidden instead and the icon shown. Whether it was;
/// if not, the window is to close as any window does. Waits for the main thread to decide.
pub fn hide_while_batching(app: &AppHandle, discard: bool) -> bool {
    let (answer, decided) = mpsc::channel();
    let on_main = app.clone();
    if app
        .run_on_main_thread(move || {
            let _ = answer.send(hide(&on_main, discard));
        })
        .is_err()
    {
        return false;
    }
    decided.recv().unwrap_or(false)
}

/// On the main thread: hides the window and shows the icon, when a batch is running.
fn hide(app: &AppHandle, discard: bool) -> bool {
    if !app.state::<Batches>().any_running() {
        return false;
    }
    let Some(window) = app.get_webview_window(WINDOW) else {
        return false;
    };
    let background = app.state::<Background>();
    let mut state = background.lock();
    if state.mode == Mode::Open {
        // No icon, no batch the user can see or stop: the window closes as it would.
        if show_icon(app, state.progress).is_err() {
            return false;
        }
        if window.hide().is_err() {
            let _ = app.remove_tray_by_id(ICON);
            return false;
        }
    }
    state.mode = Mode::Hidden { discard };
    true
}

/// The icon, with how far the batch has come, and its menu.
fn show_icon(app: &AppHandle, (done, total): (u32, u32)) -> tauri::Result<()> {
    // An icon nobody can see is no way back to a hidden window: without the app's, there is none.
    let Some(picture) = app.default_window_icon() else {
        return Err(tauri::Error::InvalidIcon(ErrorKind::NotFound.into()));
    };
    let show = MenuItem::with_id(app, SHOW, strings::BATCH_TRAY_SHOW, true, None::<&str>)?;
    let stop = MenuItem::with_id(app, STOP, strings::BATCH_TRAY_STOP, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &stop])?;
    TrayIconBuilder::with_id(ICON)
        .icon(picture.clone())
        .tooltip(strings::batch_tray_tooltip(done, total))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW => show_window(app),
            STOP => app.state::<Batches>().cancel_all(),
            _ => {}
        })
        .on_tray_icon_event(|icon, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(icon.app_handle());
            }
        })
        .build(app)
        .map(|_| ())
}

/// Shows the window and brings it to the front (the icon's menu, a second launch that hands it a
/// file): a close that was waiting for a batch is called off, and the icon goes.
pub fn show_window(app: &AppHandle) {
    let on_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        {
            let background = on_main.state::<Background>();
            let mut state = background.lock();
            if state.mode != Mode::Open {
                state.mode = Mode::Open;
                let _ = on_main.remove_tray_by_id(ICON);
            }
        }
        if let Some(window) = on_main.get_webview_window(WINDOW) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    });
}

/// What a batch did to each of its files, as it hears of them: for its report.
#[derive(Debug, Default)]
pub struct Record {
    files: Vec<String>,
    outcomes: Vec<Option<BatchOutcome>>,
}

impl Record {
    pub fn hear(&mut self, event: &BatchEvent) {
        match event {
            BatchEvent::Started { files } => {
                self.files = files.clone();
                self.outcomes = vec![None; files.len()];
            }
            BatchEvent::File { index, outcome } => {
                if let Some(slot) = usize::try_from(*index)
                    .ok()
                    .and_then(|index| self.outcomes.get_mut(index))
                {
                    *slot = Some(outcome.clone());
                }
            }
        }
    }

    /// Files done, and files.
    pub fn progress(&self) -> (u32, u32) {
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        (
            count(
                self.outcomes
                    .iter()
                    .filter(|outcome| outcome.is_some())
                    .count(),
            ),
            count(self.files.len()),
        )
    }
}

/// A batch heard of a file (or of its files): the icon, if it is shown, says how far it has come.
pub fn progress(app: &AppHandle, record: &RefCell<Record>) {
    let progress = record.borrow().progress();
    let on_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        on_main.state::<Background>().lock().progress = progress;
        if let Some(icon) = on_main.tray_by_id(ICON) {
            let (done, total) = progress;
            let _ = icon.set_tooltip(Some(strings::batch_tray_tooltip(done, total)));
        }
    });
}

/// A batch that ended: what it did, where, and how it went.
pub struct Ended {
    /// What the batch did, as the report names it.
    pub operation: &'static str,
    pub folder: PathBuf,
    pub record: Record,
    pub report: BatchReport,
}

/// A batch ended (it is not running any more). With the window hidden, its report goes into its
/// folder; once no batch is left, a message says how it went, and the app closes when the user
/// has read it.
pub fn batch_ended(app: &AppHandle, ended: Ended) {
    let on_main = app.clone();
    let _ = app.run_on_main_thread(move || ended_on_main(&on_main, ended));
}

fn ended_on_main(app: &AppHandle, ended: Ended) {
    let background = app.state::<Background>();
    let mut state = background.lock();
    let Mode::Hidden { discard } = state.mode else {
        return;
    };
    let written = write_report(&ended);
    if app.state::<Batches>().any_running() {
        // Another batch goes on: the window stays hidden until it ends.
        return;
    }
    state.mode = Mode::Closing { discard };
    drop(state);
    let _ = app.remove_tray_by_id(ICON);
    let report = written.ok().map(|path| path.display().to_string());
    let message = strings::batch_done_message(
        &strings::batch_summary(ended.report.done, ended.report.failed, ended.report.skipped),
        report.as_deref(),
    );
    let after = app.clone();
    // The message waits for the user on a thread of its own: the main thread goes on.
    std::thread::spawn(move || {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title(strings::BATCH_DONE_TITLE)
            .set_description(message)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        let on_main = after.clone();
        let _ = after.run_on_main_thread(move || close(&on_main));
    });
}

/// On the main thread, once the user has read the message: the app closes as it would have when
/// the window was closed, unless the window was shown again meanwhile.
fn close(app: &AppHandle) {
    let Mode::Closing { discard } = app.state::<Background>().lock().mode else {
        return;
    };
    if discard {
        // As closing the window does: nothing is left to recover the changes from (B2-13).
        app.state::<Documents>().discard_unsaved();
    }
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.destroy();
    }
}

/// Writes the report of `ended` into its folder, under a name no file has; its path.
fn write_report(ended: &Ended) -> std::io::Result<PathBuf> {
    let text = report_text(ended);
    for number in 1..=MAX_REPORT_NUMBER {
        let path = ended
            .folder
            .join(batch::numbered(strings::BATCH_REPORT_FILE_NAME, number));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                return Ok(path);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(ErrorKind::AlreadyExists.into())
}

/// The report of a batch: what it did, where, how it went, and each file's outcome, in lines
/// Notepad shows as lines. No password is in it: a batch's operation only gives its name.
fn report_text(ended: &Ended) -> String {
    let mut lines = vec![
        strings::BATCH_REPORT_TITLE.to_owned(),
        String::new(),
        strings::batch_report_operation(ended.operation),
        strings::batch_report_folder(&ended.folder.display().to_string()),
        strings::batch_summary(ended.report.done, ended.report.failed, ended.report.skipped),
        String::new(),
    ];
    for (name, outcome) in ended.record.files.iter().zip(&ended.record.outcomes) {
        lines.push(name.clone());
        lines.push(format!(
            "  {}",
            outcome.as_ref().map_or_else(
                || strings::BATCH_NOT_DONE.to_owned(),
                strings::batch_outcome
            )
        ));
    }
    let mut text = lines.join("\r\n");
    text.push_str("\r\n");
    text
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::ErrorCode;

    use super::*;

    fn record() -> Record {
        let mut record = Record::default();
        record.hear(&BatchEvent::Started {
            files: vec![
                "a.pdf".to_owned(),
                "密碼.pdf".to_owned(),
                "c.pdf".to_owned(),
            ],
        });
        record
    }

    #[test]
    fn a_record_hears_of_each_file_and_says_how_far_the_batch_has_come() {
        let mut record = record();
        assert_eq!(record.progress(), (0, 3));
        record.hear(&BatchEvent::File {
            index: 1,
            outcome: BatchOutcome::Failed {
                error: ErrorCode::Encrypted,
            },
        });
        assert_eq!(record.progress(), (1, 3));
        // A file it was not told of is not counted.
        record.hear(&BatchEvent::File {
            index: 7,
            outcome: BatchOutcome::Skipped,
        });
        assert_eq!(record.progress(), (1, 3));
    }

    #[test]
    fn the_report_names_each_file_and_its_outcome_and_nothing_else() {
        let mut record = record();
        for (index, outcome) in [
            BatchOutcome::Done {
                output: "a（已加密）.pdf".to_owned(),
                bates: None,
            },
            BatchOutcome::Failed {
                error: ErrorCode::Encrypted,
            },
        ]
        .into_iter()
        .enumerate()
        {
            record.hear(&BatchEvent::File {
                index: index as u32,
                outcome,
            });
        }
        let ended = Ended {
            operation: strings::batch_operation_name(
                &ipc_contract::types::BatchOperation::Encrypt {
                    open_password: Some(ipc_contract::types::Password::new("s3cret-pw".to_owned())),
                    permissions_password: None,
                    restrictions: Default::default(),
                },
            ),
            folder: PathBuf::from(r"C:\out"),
            record,
            report: BatchReport {
                done: 1,
                failed: 1,
                skipped: 0,
            },
        };
        let text = report_text(&ended);
        assert_eq!(
            text,
            [
                strings::BATCH_REPORT_TITLE.to_owned(),
                String::new(),
                strings::batch_report_operation(ended.operation),
                strings::batch_report_folder(r"C:\out"),
                strings::batch_summary(1, 1, 0),
                String::new(),
                "a.pdf".to_owned(),
                "  完成：a（已加密）.pdf".to_owned(),
                "密碼.pdf".to_owned(),
                format!(
                    "  {}",
                    strings::batch_outcome(&BatchOutcome::Failed {
                        error: ErrorCode::Encrypted
                    })
                ),
                "c.pdf".to_owned(),
                format!("  {}", strings::BATCH_NOT_DONE),
                String::new(),
            ]
            .join("\r\n")
        );
        assert!(!text.contains("s3cret-pw"));
    }

    #[test]
    fn a_report_never_replaces_a_file_and_is_numbered_when_its_name_is_taken() {
        let folder = std::env::temp_dir().join(format!("b304-report-{}", std::process::id()));
        std::fs::remove_dir_all(&folder).ok();
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join(strings::BATCH_REPORT_FILE_NAME),
            b"the user's own",
        )
        .unwrap();
        let ended = Ended {
            operation: strings::batch_operation_name(
                &ipc_contract::types::BatchOperation::RemoveMetadata,
            ),
            folder: folder.clone(),
            record: record(),
            report: BatchReport::default(),
        };
        let first = write_report(&ended).unwrap();
        let second = write_report(&ended).unwrap();
        assert_eq!(
            first,
            folder.join(batch::numbered(strings::BATCH_REPORT_FILE_NAME, 2))
        );
        assert_eq!(
            second,
            folder.join(batch::numbered(strings::BATCH_REPORT_FILE_NAME, 3))
        );
        assert_eq!(
            std::fs::read(folder.join(strings::BATCH_REPORT_FILE_NAME)).unwrap(),
            b"the user's own"
        );
        assert_eq!(
            std::fs::read_to_string(&first).unwrap(),
            report_text(&ended)
        );
        // A folder that is not there: an error, and no file.
        let gone = Ended {
            folder: folder.join("not here"),
            ..ended
        };
        assert!(write_report(&gone).is_err());
        std::fs::remove_dir_all(&folder).ok();
    }
}
