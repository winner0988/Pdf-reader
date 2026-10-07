//! The files whose pages an open document took in, or may be about to (B2-06,
//! docs/architecture/merge.md). The worker makes a clean copy of a file the user chose; the main
//! process keeps that copy here, named by a number, for as long as an edit of the document uses
//! it: the edit names the file, and every time the edit is made again (undo opens the document
//! again and makes all but the last edit again, ADR 0013) the worker is sent the copy with it.
//!
//! The crash recovery journal cannot keep a copy (it is up to 64 MiB; the journal is 4 MiB), so
//! the journal stops before the first edit that uses one (`recovery.rs`).

use std::sync::Arc;

use ipc_contract::limits::MAX_SOURCES_BYTES;
use ipc_contract::types::{Edit, SecurityFinding, SecurityReport, SourceId};

/// Most files one document keeps copies of.
pub const MAX_SOURCES: usize = 16;

/// There is no room for another copy: every one kept is used by an edit.
#[derive(Debug, PartialEq, Eq)]
pub struct Full;

/// What the worker made of a file.
#[derive(Debug)]
pub struct Source {
    /// The clean copy: a plain PDF.
    pub bytes: Vec<u8>,
    pub pages: u32,
    /// The active content the file had, which did not come along (MVP-11).
    pub security: SecurityReport,
}

/// The copies a document keeps.
#[derive(Debug, Default)]
pub struct Sources {
    kept: Vec<(SourceId, Arc<Source>)>,
    /// The number the next copy gets: never one that was used, so that a number never means two
    /// files (an edit of the history may still name an old one).
    next: u32,
}

/// The file an edit takes pages from, if it does.
fn used_by(edit: &Edit) -> Option<SourceId> {
    match edit {
        Edit::InsertPages { source, .. } => Some(*source),
        _ => None,
    }
}

impl Sources {
    pub fn pages(&self, id: SourceId) -> Option<u32> {
        self.get(id).map(|source| source.pages)
    }

    /// The copy of file `id`, for the worker.
    pub fn bytes(&self, id: SourceId) -> Option<Vec<u8>> {
        self.get(id).map(|source| source.bytes.clone())
    }

    fn get(&self, id: SourceId) -> Option<&Arc<Source>> {
        self.kept
            .iter()
            .find(|(kept, _)| *kept == id)
            .map(|(_, source)| source)
    }

    /// Keeps `source` as a new file. First the files that no edit of `history` (those done and
    /// those undone, which can be done again) uses are let go: a file that was chosen but not
    /// used is replaced by the user's next choice. If what is left, with this, is more than a
    /// document keeps, there is no room.
    pub fn add<'a>(
        &mut self,
        source: Source,
        history: impl IntoIterator<Item = &'a Edit>,
    ) -> Result<SourceId, Full> {
        let used: Vec<SourceId> = history.into_iter().filter_map(used_by).collect();
        self.kept.retain(|(id, _)| used.contains(id));
        let bytes: usize = self.kept.iter().map(|(_, kept)| kept.bytes.len()).sum();
        if self.kept.len() >= MAX_SOURCES || bytes + source.bytes.len() > MAX_SOURCES_BYTES {
            return Err(Full);
        }
        let id = SourceId(self.next);
        self.next += 1;
        self.kept.push((id, Arc::new(source)));
        Ok(id)
    }

    /// Lets every copy go: the document was written, and the file has the pages.
    pub fn clear(&mut self) {
        self.kept.clear();
    }

    /// What the files that `applied` (the edits the document has) took pages from had that is
    /// active, added to `base` (what the document's own file has): what the banner says.
    pub fn security(&self, base: &SecurityReport, applied: &[Edit]) -> SecurityReport {
        let mut report = base.clone();
        let mut seen: Vec<SourceId> = Vec::new();
        for id in applied.iter().filter_map(used_by) {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let Some(source) = self.get(id) else {
                continue;
            };
            report.scan_complete &= source.security.scan_complete;
            for finding in &source.security.findings {
                add_finding(&mut report.findings, finding);
            }
        }
        report
    }
}

/// Adds `finding` to `findings`: each kind appears once, with the sum of its counts.
fn add_finding(findings: &mut Vec<SecurityFinding>, finding: &SecurityFinding) {
    match findings.iter_mut().find(|known| known.kind == finding.kind) {
        Some(known) => known.count = known.count.saturating_add(finding.count),
        None => findings.push(*finding),
    }
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::FindingKind;

    use super::*;

    fn file(len: usize, pages: u32, findings: &[(FindingKind, u32)]) -> Source {
        Source {
            bytes: vec![0; len],
            pages,
            security: SecurityReport {
                findings: findings
                    .iter()
                    .map(|&(kind, count)| SecurityFinding { kind, count })
                    .collect(),
                scan_complete: true,
            },
        }
    }

    fn taking(source: SourceId) -> Edit {
        Edit::InsertPages { at: 0, source }
    }

    #[test]
    fn a_file_is_kept_while_an_edit_uses_it_and_its_number_is_not_given_again() {
        let mut sources = Sources::default();
        let a = sources.add(file(10, 3, &[]), []).unwrap();
        assert_eq!(sources.pages(a), Some(3));
        let history = [taking(a)];
        let b = sources.add(file(10, 5, &[]), &history).unwrap();
        assert_eq!((sources.pages(a), sources.pages(b)), (Some(3), Some(5)));
        // The edit is gone from the history (saved, or undone and replaced): so are the files.
        let c = sources.add(file(10, 1, &[]), []).unwrap();
        assert_eq!((sources.pages(a), sources.pages(b)), (None, None));
        assert!(c.0 > b.0);
        assert_eq!(sources.bytes(c), Some(vec![0; 10]));
        sources.clear();
        assert_eq!(sources.pages(c), None);
    }

    #[test]
    fn a_document_keeps_so_many_files_and_so_many_bytes() {
        let mut sources = Sources::default();
        let mut history = Vec::new();
        for _ in 0..MAX_SOURCES {
            let id = sources.add(file(1, 1, &[]), &history).unwrap();
            history.push(taking(id));
        }
        assert_eq!(sources.add(file(1, 1, &[]), &history), Err(Full));
        history.remove(0);
        assert!(sources.add(file(1, 1, &[]), &history).is_ok());

        // And bytes: two of the most one may be, in one document, do not fit with a third.
        let mut sources = Sources::default();
        let big = ipc_contract::limits::MAX_SOURCE_BYTES;
        let a = sources.add(file(big, 1, &[]), []).unwrap();
        let history = [taking(a)];
        let b = sources.add(file(big, 1, &[]), &history).unwrap();
        let history = [taking(a), taking(b)];
        assert_eq!(sources.add(file(1, 1, &[]), &history), Err(Full));
    }

    #[test]
    fn the_banner_has_what_the_files_taken_from_had_and_what_the_file_has() {
        let mut sources = Sources::default();
        let a = sources
            .add(
                file(
                    1,
                    1,
                    &[(FindingKind::JavaScript, 2), (FindingKind::Launch, 1)],
                ),
                [],
            )
            .unwrap();
        let history = [taking(a)];
        let b = sources
            .add(file(1, 1, &[(FindingKind::JavaScript, 3)]), &history)
            .unwrap();
        let own = SecurityReport {
            findings: vec![SecurityFinding {
                kind: FindingKind::JavaScript,
                count: 1,
            }],
            scan_complete: true,
        };
        // Nothing taken: the file's own.
        assert_eq!(sources.security(&own, &[]), own);
        // Counted once for a file used twice, summed over the files and the document's own.
        let applied = [taking(a), taking(a), taking(b)];
        let report = sources.security(&own, &applied);
        let count = |kind| {
            report
                .findings
                .iter()
                .find(|finding| finding.kind == kind)
                .map(|finding| finding.count)
        };
        assert_eq!(count(FindingKind::JavaScript), Some(1 + 2 + 3));
        assert_eq!(count(FindingKind::Launch), Some(1));
        assert!(report.scan_complete);
        // A scan that did not finish is told on.
        let mut unfinished = file(1, 1, &[]);
        unfinished.security.scan_complete = false;
        let c = sources.add(unfinished, &[taking(a), taking(b)]).unwrap();
        assert!(!sources.security(&own, &[taking(c)]).scan_complete);
    }
}
