//! Batches (B3-04, ADR 0005, docs/architecture/batch.md): one operation done to many files, one
//! after the other. Each file is opened in a worker of its own (ADR 0012), which ends when its copy
//! is written; the copy goes to the folder the user chose, under a name no file has, so that no
//! file is ever changed or replaced. A file that fails does not stop the others, and the batch
//! can be stopped before its next file. Paths stay here: the page hears of the files by name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ipc_contract::types::{BatchEvent, BatchOperation, BatchOutcome, BatchReport, RequestId};

use crate::documents::{self, Documents};
use crate::export;
use crate::strings;

/// The batches that are running, each with what stops it.
#[derive(Default)]
pub struct Batches {
    running: Mutex<HashMap<RequestId, Arc<AtomicBool>>>,
}

impl Batches {
    /// Stops the batch `request` before its next file. Unknown requests are ignored.
    pub fn cancel(&self, request: RequestId) {
        if let Some(stop) = self.lock().get(&request) {
            stop.store(true, Ordering::SeqCst);
        }
    }

    /// Marks `request` as running until the guard is dropped, and gives what stops it.
    pub fn start(&self, request: RequestId) -> Running<'_> {
        let stop = Arc::new(AtomicBool::new(false));
        self.lock().insert(request, stop.clone());
        Running {
            batches: self,
            request,
            stop,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<RequestId, Arc<AtomicBool>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A batch that is running; it is not any more when this is dropped.
pub struct Running<'a> {
    batches: &'a Batches,
    request: RequestId,
    stop: Arc<AtomicBool>,
}

impl Running<'_> {
    pub fn stop(&self) -> &AtomicBool {
        &self.stop
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.batches.lock().remove(&self.request);
    }
}

/// What the copy of the file `display_name` is called (before a number makes it a name no file
/// has): what the privacy export or the encrypted copy of one document suggests.
pub fn copy_name(display_name: &str, operation: &BatchOperation) -> String {
    let stem = export::stem(display_name);
    match operation {
        BatchOperation::RemoveMetadata => strings::privacy_export_file_name(&stem),
        BatchOperation::Encrypt { .. } => strings::encrypted_copy_file_name(&stem),
    }
}

/// `name` with `number` before its extension, from 2 on: `a.pdf`, `a (2).pdf`, `a (3).pdf`.
pub fn numbered(name: &str, number: u32) -> String {
    if number <= 1 {
        return name.to_owned();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({number}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({number})"),
    }
}

/// Does `operation` to each of `files` in turn and writes the copies to `folder`. `report` hears
/// of the files first, then of each as it is done; once `stop` is set, the files not yet begun
/// are skipped.
pub fn run(
    documents: &Documents,
    files: &[PathBuf],
    folder: &Path,
    operation: &BatchOperation,
    stop: &AtomicBool,
    report: &dyn Fn(BatchEvent),
) -> BatchReport {
    let names: Vec<String> = files
        .iter()
        .map(|path| documents::display_name(path))
        .collect();
    report(BatchEvent::Started {
        files: names.clone(),
    });
    let mut summary = BatchReport::default();
    for (index, (path, name)) in files.iter().zip(&names).enumerate() {
        let outcome = if stop.load(Ordering::SeqCst) {
            summary.skipped += 1;
            BatchOutcome::Skipped
        } else {
            match documents.batch_copy(path, folder, &copy_name(name, operation), operation) {
                Ok(output) => {
                    summary.done += 1;
                    BatchOutcome::Done { output }
                }
                Err(error) => {
                    summary.failed += 1;
                    BatchOutcome::Failed { error: error.code }
                }
            }
        };
        report(BatchEvent::File {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            outcome,
        });
    }
    summary
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::{Password, Restrictions};

    use super::*;

    #[test]
    fn a_copy_is_named_as_the_one_of_a_single_document_is_and_numbered_when_taken() {
        assert_eq!(
            copy_name("報告.pdf", &BatchOperation::RemoveMetadata),
            "報告（隱私匯出）.pdf"
        );
        let encrypt = BatchOperation::Encrypt {
            open_password: Some(Password::new("pw".to_owned())),
            permissions_password: None,
            restrictions: Restrictions::default(),
        };
        assert_eq!(copy_name("Report.PDF", &encrypt), "Report（已加密）.pdf");
        assert_eq!(numbered("a.pdf", 1), "a.pdf");
        assert_eq!(numbered("a.pdf", 2), "a (2).pdf");
        assert_eq!(
            numbered("報告（已加密）.pdf", 10),
            "報告（已加密） (10).pdf"
        );
        assert_eq!(numbered("noextension", 3), "noextension (3)");
        assert_eq!(numbered(".pdf", 2), ".pdf (2)");
    }

    #[test]
    fn a_batch_is_known_until_it_ends_and_can_be_stopped() {
        let batches = Batches::default();
        {
            let running = batches.start(RequestId(4));
            assert_eq!(batches.lock().len(), 1);
            assert!(!running.stop().load(Ordering::SeqCst));
            batches.cancel(RequestId(5));
            assert!(!running.stop().load(Ordering::SeqCst));
            batches.cancel(RequestId(4));
            assert!(running.stop().load(Ordering::SeqCst));
        }
        assert!(batches.lock().is_empty());
        // Stopping one that is not running does nothing.
        batches.cancel(RequestId(4));
        assert!(batches.lock().is_empty());
    }
}
