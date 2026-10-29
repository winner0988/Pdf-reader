//! Form field scripts execution via MuPDF's built-in mujs (B3-07, ADR 0021 POC).
//!
//! # Safety
//! All `unsafe` code interfacing with MuPDF C FFI for form scripts is concentrated in this module.
//! Each `unsafe` block documents its safety invariants.
//!
//! The PDF engine runs in a low-privilege sandboxed worker process. When JavaScript is enabled,
//! mujs executes within this sandbox. Document events that would cause side effects outside the
//! worker (alert, print, launchURL, mailDoc, submitForm, execMenuItem) are intercepted and
//! blocked via `pdf_set_doc_event_callback`.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::ptr;

use ipc_contract::limits::{MAX_BLOCKED_ACTIONS, MAX_SCRIPT_BYTES};
use ipc_contract::types::FieldId;
use ipc_contract::worker::{ChangedField, FormFieldEvent};
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject};
use mupdf_sys::{
    fz_context, fz_document, fz_free, pdf_calculate_form, pdf_doc_event, pdf_document,
    pdf_field_event_calculate, pdf_field_event_format, pdf_field_event_keystroke,
    pdf_field_event_validate, pdf_keystroke_event, pdf_obj, pdf_set_doc_event_callback,
    pdf_set_field_value, PDF_DOCUMENT_EVENT_ALERT, PDF_DOCUMENT_EVENT_EXEC_MENU_ITEM,
    PDF_DOCUMENT_EVENT_LAUNCH_URL, PDF_DOCUMENT_EVENT_MAIL_DOC, PDF_DOCUMENT_EVENT_PRINT,
    PDF_DOCUMENT_EVENT_SUBMIT,
};

use super::{EngineError, PdfDocument};

thread_local! {
    static BLOCKED_ACTIONS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Outcome of running a form field event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldEventOutcome {
    pub accepted: bool,
    pub value: Option<String>,
    pub changed_fields: Vec<ChangedField>,
    pub blocked_actions: Vec<String>,
}

/// Callback registered with MuPDF to intercept all document events emanating from scripts.
unsafe extern "C" fn doc_event_callback(
    ctx: *mut fz_context,
    _doc: *mut pdf_document,
    evt: *mut pdf_doc_event,
    _data: *mut std::ffi::c_void,
) {
    if evt.is_null() {
        return;
    }
    // SAFETY: evt is valid for the duration of the callback as passed by MuPDF.
    let event_type = unsafe { (*evt).type_ };
    let name = match event_type {
        PDF_DOCUMENT_EVENT_ALERT => {
            // Intercept alert dialogs: provide default OK button press so the script continues
            // without expecting user response.
            // SAFETY: evt has type ALERT, so pdf_access_alert_event returns a valid pointer.
            let alert = unsafe { mupdf_sys::pdf_access_alert_event(ctx, evt) };
            if !alert.is_null() {
                unsafe {
                    (*alert).button_pressed = mupdf_sys::PDF_ALERT_BUTTON_OK;
                }
            }
            "alert"
        }
        PDF_DOCUMENT_EVENT_PRINT => "print",
        PDF_DOCUMENT_EVENT_LAUNCH_URL => "launchURL",
        PDF_DOCUMENT_EVENT_MAIL_DOC => "mailDoc",
        PDF_DOCUMENT_EVENT_SUBMIT => "submitForm",
        PDF_DOCUMENT_EVENT_EXEC_MENU_ITEM => "execMenuItem",
        _ => "unknown",
    };
    BLOCKED_ACTIONS.with(|cell| {
        let mut list = cell.borrow_mut();
        if list.len() < MAX_BLOCKED_ACTIONS {
            list.push(name.to_owned());
        }
    });
}

// Memory layout assertions for raw pointer access
const _: () = {
    assert!(size_of::<mupdf::Document>() == size_of::<*mut fz_document>());
    assert!(align_of::<mupdf::Document>() == align_of::<*mut fz_document>());
    assert!(size_of::<mupdf::Context>() == size_of::<*mut fz_context>());
    assert!(align_of::<mupdf::Context>() == align_of::<*mut fz_context>());
    assert!(size_of::<PdfObject>() == size_of::<*mut pdf_obj>());
    assert!(align_of::<PdfObject>() == align_of::<*mut pdf_obj>());
};

/// Reads the raw underlying `pdf_document` pointer from a `MuPdfDocument`.
///
/// # Safety
/// `MuPdfDocument` dereferences to `mupdf::Document`, which is a single-field wrapper around
/// `*mut fz_document`. `pdf_document_from_fz_document` safely extracts the underlying
/// `*mut pdf_document`. The returned pointer is valid as long as `doc` is alive.
#[inline]
unsafe fn raw_doc(ctx: *mut fz_context, doc: &MuPdfDocument) -> *mut pdf_document {
    let fz_doc: *mut mupdf_sys::fz_document = unsafe {
        let d: &mupdf::Document = doc;
        *(ptr::from_ref(d).cast::<*mut mupdf_sys::fz_document>())
    };
    unsafe { mupdf_sys::pdf_document_from_fz_document(ctx, fz_doc) }
}

/// Reads the raw underlying `fz_context` pointer from a `mupdf::Context`.
///
/// # Safety
/// `mupdf::Context` is guaranteed to have the same layout as `*mut fz_context`.
/// The returned pointer is valid as long as the thread-local context is alive.
#[inline]
unsafe fn raw_context(ctx: &mupdf::Context) -> *mut fz_context {
    unsafe { *(ptr::from_ref(ctx).cast::<*mut fz_context>()) }
}

/// Reads the raw underlying `pdf_obj` pointer from a `PdfObject`.
///
/// # Safety
/// `PdfObject` is guaranteed to have the same layout as `*mut pdf_obj`.
/// The returned pointer is valid as long as `obj` is alive.
#[inline]
unsafe fn raw_obj(obj: &PdfObject) -> *mut pdf_obj {
    unsafe { *(ptr::from_ref(obj).cast::<*mut pdf_obj>()) }
}

/// Calculates the total bytes of JavaScript scripts present in the document.
pub fn total_script_bytes(catalog: &PdfObject) -> Result<usize, EngineError> {
    let mut total = 0usize;
    let mut seen = HashSet::new();
    let mut stack = vec![catalog.try_clone()?];
    let mut budget = 100_000usize;

    while let Some(obj) = stack.pop() {
        if budget == 0 {
            break;
        }
        budget -= 1;

        if obj.is_indirect()? {
            let num = obj.as_indirect()?;
            if !seen.insert(num) {
                continue;
            }
        }

        if obj.is_array()? {
            for item in obj.array_iter()? {
                let item = item?;
                if item.is_indirect()? || item.is_dict()? || item.is_array()? {
                    stack.push(item);
                }
            }
            continue;
        }

        if !obj.is_dict()? {
            continue;
        }

        if let Some(js) = obj.get_dict("JS")? {
            if js.is_string()? {
                total = total.saturating_add(js.as_bytes()?.len());
            } else if js.is_stream()? {
                total = total.saturating_add(js.read_raw_stream()?.len());
            }
        }

        for entry in obj.dict_iter()? {
            let (_, value) = entry?;
            if value.is_indirect()? || value.is_dict()? || value.is_array()? {
                stack.push(value);
            }
        }
    }

    Ok(total)
}

impl PdfDocument {
    /// Explicitly enables JavaScript for this document (B3-07, ADR 0021).
    ///
    /// Documents open with JavaScript disabled. Scripts are enabled only when the user explicitly
    /// triggers an interaction on a scripted field. If total script bytes exceed 1 MB, enablement
    /// is refused.
    pub fn enable_js(&mut self) -> Result<(), EngineError> {
        let catalog = self.doc.catalog()?;
        let total = total_script_bytes(&catalog)?;
        if total > MAX_SCRIPT_BYTES {
            return Err(EngineError::InvalidEdit("form scripts exceed 1 MB limit"));
        }

        self.doc.enable_js()?;

        // Register document event callback to catch and block all outward actions (alert, print, etc.)
        let ctx_handle = mupdf::Context::get();
        // SAFETY: ctx and doc pointers are valid while the document is open on this thread.
        unsafe {
            let ctx = raw_context(&ctx_handle);
            let pdoc = raw_doc(ctx, &self.doc);
            pdf_set_doc_event_callback(
                ctx,
                pdoc,
                Some(doc_event_callback),
                None,
                ptr::null_mut(),
            );
        }

        Ok(())
    }

    /// Dispatches a form field event (Keystroke, Format, Validate, Calculate) to `field` on `page`.
    pub fn dispatch_field_event(
        &mut self,
        page_index: u32,
        field: FieldId,
        event: &FormFieldEvent,
    ) -> Result<FieldEventOutcome, EngineError> {
        if !self.doc.is_js_supported()? {
            return Err(EngineError::InvalidEdit(
                "JavaScript is not enabled for this document",
            ));
        }

        let before = self.collect_field_values()?;
        BLOCKED_ACTIONS.with(|cell| cell.borrow_mut().clear());

        let page = self.pdf_page(page_index)?;
        let wanted = i32::try_from(field.0).map_err(|_| super::forms::no_such_field())?;
        let widget = page
            .widgets()
            .find(|w| w.xref().is_ok_and(|number| number == wanted))
            .ok_or_else(super::forms::no_such_field)?;

        let object = widget.annotation().object();
        let ctx_handle = mupdf::Context::get();

        let (accepted, value) = match event {
            FormFieldEvent::Keystroke {
                value,
                change,
                sel_start,
                sel_end,
                will_commit,
            } => {
                let c_val = CString::new(value.as_str())
                    .map_err(|_| EngineError::InvalidEdit("nul byte in value"))?;
                let c_change = CString::new(change.as_str())
                    .map_err(|_| EngineError::InvalidEdit("nul byte in change"))?;
                let mut evt = pdf_keystroke_event {
                    value: c_val.as_ptr(),
                    change: c_change.as_ptr(),
                    selStart: *sel_start,
                    selEnd: *sel_end,
                    willCommit: if *will_commit { 1 } else { 0 },
                    newChange: ptr::null_mut(),
                    newValue: ptr::null_mut(),
                };

                // SAFETY: ctx, pdoc, pobj, and evt references are valid and live for the call.
                let ret = unsafe {
                    let ctx = raw_context(&ctx_handle);
                    let pdoc = raw_doc(ctx, &self.doc);
                    let pobj = raw_obj(&object);
                    pdf_field_event_keystroke(ctx, pdoc, pobj, &mut evt)
                };

                let accepted = ret != 0;
                let new_value = if !evt.newValue.is_null() {
                    // SAFETY: MuPDF allocated evt.newValue using fz_strdup; we read it then free it.
                    let s = unsafe { CStr::from_ptr(evt.newValue) }
                        .to_string_lossy()
                        .into_owned();
                    unsafe {
                        let ctx = raw_context(&ctx_handle);
                        fz_free(ctx, evt.newValue.cast());
                    }
                    Some(s)
                } else {
                    None
                };
                if !evt.newChange.is_null() {
                    // SAFETY: MuPDF allocated evt.newChange using fz_strdup; we free it.
                    unsafe {
                        let ctx = raw_context(&ctx_handle);
                        fz_free(ctx, evt.newChange.cast());
                    }
                }
                (accepted, new_value)
            }
            FormFieldEvent::Format => {
                // SAFETY: ctx, pdoc, and pobj references are valid and live for the call.
                let ptr = unsafe {
                    let ctx = raw_context(&ctx_handle);
                    let pdoc = raw_doc(ctx, &self.doc);
                    let pobj = raw_obj(&object);
                    pdf_field_event_format(ctx, pdoc, pobj)
                };

                if !ptr.is_null() {
                    // SAFETY: MuPDF allocated ptr with fz_strdup; we read it and free it.
                    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
                    unsafe {
                        let ctx = raw_context(&ctx_handle);
                        fz_free(ctx, ptr.cast());
                    }
                    (true, Some(s))
                } else {
                    (true, None)
                }
            }
            FormFieldEvent::Validate { value } => {
                let c_val = CString::new(value.as_str())
                    .map_err(|_| EngineError::InvalidEdit("nul byte in value"))?;
                let mut newvalue: *mut std::ffi::c_char = ptr::null_mut();

                // SAFETY: ctx, pdoc, pobj, and newvalue pointer are valid for the call.
                let ret = unsafe {
                    let ctx = raw_context(&ctx_handle);
                    let pdoc = raw_doc(ctx, &self.doc);
                    let pobj = raw_obj(&object);
                    pdf_field_event_validate(ctx, pdoc, pobj, c_val.as_ptr(), &mut newvalue)
                };

                let accepted = ret != 0;
                let val_to_set = if !newvalue.is_null() {
                    // SAFETY: MuPDF allocated newvalue; we read it then free it.
                    let s = unsafe { CStr::from_ptr(newvalue) }
                        .to_string_lossy()
                        .into_owned();
                    unsafe {
                        let ctx = raw_context(&ctx_handle);
                        fz_free(ctx, newvalue.cast());
                    }
                    s
                } else {
                    value.clone()
                };

                if accepted {
                    let c_to_set = CString::new(val_to_set.as_str())
                        .map_err(|_| EngineError::InvalidEdit("nul byte in value"))?;
                    // SAFETY: ctx, pdoc, and pobj pointers are valid. We update the field value.
                    unsafe {
                        let ctx = raw_context(&ctx_handle);
                        let pdoc = raw_doc(ctx, &self.doc);
                        let pobj = raw_obj(&object);
                        pdf_set_field_value(ctx, pdoc, pobj, c_to_set.as_ptr(), 1);
                    }
                }

                (accepted, Some(val_to_set))
            }
            FormFieldEvent::Calculate => {
                // SAFETY: ctx, pdoc, and pobj pointers are valid. MuPDF calculates all form fields
                // according to the AcroForm /CO array order.
                unsafe {
                    let ctx = raw_context(&ctx_handle);
                    let pdoc = raw_doc(ctx, &self.doc);
                    let pobj = raw_obj(&object);
                    pdf_field_event_calculate(ctx, pdoc, pobj);
                    pdf_calculate_form(ctx, pdoc);
                }
                (true, None)
            }
        };

        // Update widget appearances so that visual rendering reflects any newly computed values.
        let page_count = self.page_count()?;
        for p in 0..page_count {
            let page = self.pdf_page(p)?;
            for mut w in page.widgets() {
                let _ = w.update();
            }
        }

        let after = self.collect_field_values()?;
        let changed_fields = diff_fields(&before, &after);
        let blocked_actions = BLOCKED_ACTIONS.with(|cell| cell.borrow().clone());

        Ok(FieldEventOutcome {
            accepted,
            value,
            changed_fields,
            blocked_actions,
        })
    }

    /// Collects the current values of all form fields in the document.
    pub fn collect_field_values(&self) -> Result<Vec<(FieldId, String)>, EngineError> {
        let mut list = Vec::new();
        let pages = self.page_count()?;
        for p in 0..pages {
            for f in self.page_fields(p)? {
                list.push((f.id, f.value));
            }
        }
        Ok(list)
    }
}

fn diff_fields(before: &[(FieldId, String)], after: &[(FieldId, String)]) -> Vec<ChangedField> {
    let before_map: HashMap<FieldId, &str> =
        before.iter().map(|(id, v)| (*id, v.as_str())).collect();
    let mut changed = Vec::new();
    for (id, val) in after {
        if before_map.get(id).copied() != Some(val.as_str()) {
            changed.push(ChangedField {
                id: *id,
                value: val.clone(),
            });
        }
    }
    changed
}
