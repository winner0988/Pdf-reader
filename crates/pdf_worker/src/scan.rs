//! Scan for active content and remote references (MVP-11, docs/architecture/active-content.md).
//!
//! Nothing found here is ever run or followed: MuPDF is built without JavaScript, and the viewer
//! performs no PDF actions except jumps inside the document. The scan only tells the user what
//! was blocked.
//!
//! It walks every object reachable from the catalog with an explicit stack, so deep nesting
//! cannot overflow the stack. Each indirect object is visited once, which also ends cycles. No
//! stream data is read. The walk stops at a time and object budget, and the report then says
//! the scan is incomplete.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use ipc_contract::text::is_network_path;
use ipc_contract::types::{FindingKind, SecurityFinding, SecurityReport};
use mupdf::pdf::PdfObject;

/// Limits of one scan. The scan runs while a document opens, so it must not hold it up for long.
#[derive(Debug, Clone, Copy)]
pub struct ScanBudget {
    /// Objects visited (dictionaries, arrays and references to them).
    pub max_objects: usize,
    pub max_time: Duration,
}

impl Default for ScanBudget {
    fn default() -> Self {
        Self {
            max_objects: 2_000_000,
            max_time: Duration::from_secs(2),
        }
    }
}

/// How often (in objects) the clock is read.
const CLOCK_EVERY: usize = 1024;

/// Keys of a file specification, or of an action or stream, that hold a file name.
const FILE_NAME_KEYS: [&[u8]; 5] = [b"F", b"UF", b"DOS", b"Unix", b"Mac"];

/// Annotation subtypes that embed media or interactive players.
const MEDIA_SUBTYPES: [&[u8]; 5] = [b"RichMedia", b"Screen", b"Movie", b"Sound", b"3D"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Any,
    /// The file of an action that is reported as that action (a form's submit address, for
    /// example): it is not reported again as a remote file.
    ActionFile,
}

#[derive(Default)]
struct Tally {
    counts: [u32; FindingKind::ALL.len()],
}

impl Tally {
    fn add(&mut self, kind: FindingKind, count: u32) {
        let index = FindingKind::ALL
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap_or_default();
        self.counts[index] = self.counts[index].saturating_add(count);
    }

    fn report(&self, scan_complete: bool) -> SecurityReport {
        let findings = FindingKind::ALL
            .iter()
            .zip(self.counts)
            .filter(|(_, count)| *count > 0)
            .map(|(kind, count)| SecurityFinding { kind: *kind, count })
            .collect();
        SecurityReport {
            findings,
            scan_complete,
        }
    }
}

/// Scans the document whose catalog is `catalog`.
pub fn scan(catalog: PdfObject, budget: ScanBudget) -> SecurityReport {
    let started = Instant::now();
    let mut tally = Tally::default();
    if open_action_runs_something(&catalog).unwrap_or(false) {
        tally.add(FindingKind::OpenAction, 1);
    }
    let mut seen = HashSet::new();
    let mut stack = vec![(catalog, Role::Any)];
    let mut visited = 0usize;
    while let Some((object, role)) = stack.pop() {
        visited += 1;
        if visited > budget.max_objects
            || (visited.is_multiple_of(CLOCK_EVERY) && started.elapsed() > budget.max_time)
        {
            return tally.report(false);
        }
        // A broken object only loses itself.
        let _ = visit(&object, role, &mut tally, &mut seen, &mut stack);
    }
    tally.report(true)
}

/// A catalog /OpenAction that is more than "open at this page".
fn open_action_runs_something(catalog: &PdfObject) -> Result<bool, mupdf::Error> {
    let Some(action) = catalog.get_dict("OpenAction")? else {
        return Ok(false);
    };
    // An array is a destination: the document opens at a page.
    if !action.is_dict()? {
        return Ok(false);
    }
    let plain_jump = name(action.get_dict("S")?.as_ref())?.as_deref() == Some(b"GoTo")
        && action.get_dict("Next")?.is_none();
    Ok(!plain_jump)
}

fn visit(
    object: &PdfObject,
    role: Role,
    tally: &mut Tally,
    seen: &mut HashSet<i32>,
    stack: &mut Vec<(PdfObject, Role)>,
) -> Result<(), mupdf::Error> {
    if object.is_indirect()? && !seen.insert(object.as_indirect()?) {
        return Ok(());
    }
    if object.is_array()? {
        for item in object.array_iter()? {
            let item = item?;
            if may_contain_more(&item)? {
                stack.push((item, Role::Any));
            }
        }
        return Ok(());
    }
    if !object.is_dict()? {
        return Ok(());
    }
    // One pass over the entries: looking keys up one by one costs a MuPDF call (and a new name
    // object) per key, which is most of the scan time for large outlines.
    let mut entries = Vec::new();
    for entry in object.dict_iter()? {
        let (key, value) = entry?;
        entries.push((key.as_name()?, value));
    }
    let owns_file = classify(&entries, role, tally)?;
    for (key, value) in entries {
        if !may_contain_more(&value)? {
            continue;
        }
        let role = if owns_file && key == b"F" {
            Role::ActionFile
        } else {
            Role::Any
        };
        stack.push((value, role));
    }
    Ok(())
}

/// Whether `object` may lead to more dictionaries. A reference is not loaded until it is visited.
fn may_contain_more(object: &PdfObject) -> Result<bool, mupdf::Error> {
    Ok(object.is_indirect()? || object.is_dict()? || object.is_array()?)
}

/// Records what a dictionary with these entries is. Returns true for an action whose /F is part
/// of that action.
fn classify(
    entries: &[(Vec<u8>, PdfObject)],
    role: Role,
    tally: &mut Tally,
) -> Result<bool, mupdf::Error> {
    let get = |key: &[u8]| {
        entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    };
    let action = name(get(b"S"))?;
    let mut owns_file = false;
    let mut action_kind = |kind: FindingKind, file: bool| {
        tally.add(kind, 1);
        owns_file = file;
    };
    match action.as_deref() {
        Some(b"JavaScript") => action_kind(FindingKind::JavaScript, false),
        Some(b"Launch") => action_kind(FindingKind::Launch, true),
        Some(b"SubmitForm") => action_kind(FindingKind::SubmitForm, true),
        Some(b"ImportData") => action_kind(FindingKind::ImportData, true),
        Some(b"GoToR") => action_kind(FindingKind::RemoteGoTo, true),
        Some(b"GoToE") => action_kind(FindingKind::EmbeddedGoTo, true),
        Some(b"RichMediaExecute") => action_kind(FindingKind::RichMedia, false),
        // Other actions (a rendition, for example) may carry a script too.
        _ if get(b"JS").is_some() => action_kind(FindingKind::JavaScript, false),
        _ => {}
    }

    // Triggers of the document, a page, an annotation or a form field: one per entry.
    if let Some(triggers) = get(b"AA")
        && triggers.is_dict()?
    {
        let mut count = 0u32;
        for entry in triggers.dict_iter()? {
            if entry?.1.is_dict()? {
                count += 1;
            }
        }
        tally.add(FindingKind::AdditionalActions, count);
    }
    if get(b"XFA").is_some() {
        tally.add(FindingKind::Xfa, 1);
    }
    if let Some(subtype) = name(get(b"Subtype"))?
        && MEDIA_SUBTYPES.contains(&subtype.as_slice())
    {
        tally.add(FindingKind::RichMedia, 1);
    }

    // File specifications, and file names in actions and streams.
    if get(b"EF").is_some() {
        tally.add(FindingKind::EmbeddedFile, 1);
    }
    if role == Role::Any && name(get(b"FS"))?.as_deref() == Some(b"URL") {
        tally.add(FindingKind::RemoteFileSpec, 1);
    }
    for (key, value) in entries {
        if FILE_NAME_KEYS.contains(&key.as_slice())
            && value.is_string()?
            && is_network_path(&value.as_bytes()?)
        {
            tally.add(FindingKind::UncReference, 1);
            break;
        }
    }
    Ok(owns_file)
}

fn name(object: Option<&PdfObject>) -> Result<Option<Vec<u8>>, mupdf::Error> {
    match object {
        Some(object) if object.is_name()? => Ok(Some(object.as_name()?)),
        _ => Ok(None),
    }
}
