//! Form fields (B2-09, docs/architecture/forms.md): listing a page's fields, filling them in, and
//! flattening the form into page content.
//!
//! Nothing is ever run. MuPDF is built without JavaScript; a field is only told that its value
//! changed (`ignore_trigger_events`), and a field's scripts only make `FormField::has_script` true,
//! so the page can say that they will not run.

use ipc_contract::limits::{
    MAX_FIELD_OPTIONS, MAX_FIELD_VALUE_BYTES, MAX_FIELDS_PER_PAGE, MAX_TEXT_BYTES,
};
use ipc_contract::text::{clean_display_text, clean_field_text};
use ipc_contract::types::{FieldId, FieldKind, FieldOption, FormField, Rect};
use mupdf::pdf::{AnnotationFlags, FieldFlags, PdfObject, PdfWidget, WidgetType};

use super::{EngineError, PdfDocument};

/// How far up the chain of parent fields, and down the kids of a field group, to go: a cycle or
/// an absurd depth is cut.
const MAX_FIELD_DEPTH: usize = 64;

/// Most widgets set when a radio group or check box group takes a new state.
const MAX_GROUP_WIDGETS: usize = 100_000;

impl PdfDocument {
    /// The form fields of page `index` that the app fills in: text fields, check boxes, radio
    /// buttons, combo boxes and list boxes. Hidden fields, push buttons and signature fields are
    /// not listed. At most `MAX_FIELDS_PER_PAGE`; one that cannot be read is skipped.
    pub fn page_fields(&self, index: u32) -> Result<Vec<FormField>, EngineError> {
        let page = self.pdf_page(index)?;
        let mut fields = Vec::new();
        for widget in page.widgets() {
            if fields.len() == MAX_FIELDS_PER_PAGE as usize {
                break;
            }
            if let Ok(Some(field)) = read_field(&widget) {
                fields.push(field);
            }
        }
        Ok(fields)
    }

    /// Sets the value of field `field` of page `index` (see `Edit::SetFieldValue`). A field the
    /// page does not list, one that cannot be changed, and a value the field cannot have (not an
    /// option of a list, too long, several lines in one line, a check box neither on nor off)
    /// change nothing.
    pub fn set_field_value(
        &mut self,
        index: u32,
        field: FieldId,
        value: &str,
    ) -> Result<(), EngineError> {
        let page = self.pdf_page(index)?;
        let wanted = i32::try_from(field.0).map_err(|_| no_such_field())?;
        let mut widget = page
            .widgets()
            .find(|widget| widget.xref().is_ok_and(|number| number == wanted))
            .ok_or_else(no_such_field)?;
        let listed = read_field(&widget)?.ok_or_else(no_such_field)?;
        if listed.read_only {
            return Err(EngineError::InvalidEdit("the field cannot be changed"));
        }
        let object = widget.annotation().object();
        match listed.kind {
            FieldKind::Text => {
                if !listed.multiline && value.contains('\n') {
                    return Err(EngineError::InvalidEdit("the field has one line"));
                }
                if let Some(max) = listed.max_len
                    && value.chars().count() > max as usize
                {
                    return Err(EngineError::InvalidEdit("longer than the field allows"));
                }
                self.fill(&mut widget, value)
            }
            FieldKind::Combo | FieldKind::List => {
                // The value stored is the option's own, as the file has it.
                let stored = match raw_options(&object)?
                    .into_iter()
                    .find(|(_, option)| option.value == value)
                {
                    Some((raw, _)) => raw,
                    None if listed.editable || value.is_empty() => value.to_owned(),
                    None => return Err(EngineError::InvalidEdit("not one of the field's options")),
                };
                self.fill(&mut widget, &stored)
            }
            FieldKind::Checkbox | FieldKind::Radio => {
                let on = listed.on_value.as_deref();
                let allowed =
                    on == Some(value) || (listed.kind == FieldKind::Checkbox && value == "Off");
                if !allowed {
                    return Err(EngineError::InvalidEdit("neither on nor off"));
                }
                set_button_state(&object, value)
            }
        }
    }

    /// Turns the form into page content: each field's appearance becomes part of its page, and
    /// the fields and the form go. A signed document is refused: that would make its signatures
    /// worthless.
    pub fn flatten_form(&mut self) -> Result<(), EngineError> {
        if self.is_signed() {
            return Err(EngineError::InvalidEdit(
                "a signed document cannot be flattened",
            ));
        }
        // Annotations stay what they are; only the widgets are baked. MuPDF takes the fields and
        // the form (`/AcroForm`) out with them, so a rewrite leaves out the fields, and with them
        // what is now only on the pages.
        self.doc.bake(false, true)?;
        Ok(())
    }

    /// Writes `value` into the field `widget` and draws it again.
    fn fill(&mut self, widget: &mut PdfWidget, value: &str) -> Result<(), EngineError> {
        // `true`: no script is told of it. There is none to run, and none would be.
        widget.set_value(&mut self.doc, value, true)?;
        widget.update()?;
        Ok(())
    }
}

fn no_such_field() -> EngineError {
    EngineError::InvalidEdit("no such field on the page")
}

/// `widget` as the page lists it; `None` for what it does not list.
fn read_field(widget: &PdfWidget) -> Result<Option<FormField>, EngineError> {
    let annotation = widget.annotation();
    if annotation.flags()?.intersects(
        AnnotationFlags::IS_HIDDEN | AnnotationFlags::NO_VIEW | AnnotationFlags::IS_INVISIBLE,
    ) {
        return Ok(None);
    }
    let kind = match widget.r#type()? {
        WidgetType::Text => FieldKind::Text,
        WidgetType::Checkbox => FieldKind::Checkbox,
        WidgetType::RadioButton => FieldKind::Radio,
        WidgetType::Combobox => FieldKind::Combo,
        WidgetType::Listbox => FieldKind::List,
        _ => return Ok(None),
    };
    let Some(id) = u32::try_from(widget.xref()?)
        .ok()
        .filter(|&number| number > 0)
    else {
        return Ok(None);
    };
    let flags = widget.field_flags()?;
    let object = annotation.object();
    let rect = annotation.rect()?;
    let rect = Rect {
        x0: rect.x0.min(rect.x1),
        y0: rect.y0.min(rect.y1),
        x1: rect.x0.max(rect.x1),
        y1: rect.y0.max(rect.y1),
    };
    if [rect.x0, rect.y0, rect.x1, rect.y1]
        .iter()
        .any(|value| !value.is_finite())
    {
        return Ok(None);
    }

    let label = widget
        .label()?
        .filter(|tooltip| !tooltip.trim().is_empty())
        .or(widget.name()?)
        .map(|text| clean_display_text(&text, MAX_TEXT_BYTES as usize))
        .filter(|text| !text.is_empty());

    let multiline = kind == FieldKind::Text && flags.contains(FieldFlags::MULTILINE);
    let mut cut = false;
    let (value, on_value) = match kind {
        FieldKind::Checkbox | FieldKind::Radio => (button_state(&object)?, on_state(&object)?),
        _ => {
            let raw = widget.value()?.unwrap_or_default();
            let (value, was_cut) =
                clean_field_text(&raw, MAX_FIELD_VALUE_BYTES as usize, multiline);
            cut = was_cut;
            (value, None)
        }
    };

    let options = match kind {
        FieldKind::Combo | FieldKind::List => {
            let all = raw_options(&object)?;
            cut |= all.len() > MAX_FIELD_OPTIONS as usize;
            all.into_iter()
                .take(MAX_FIELD_OPTIONS as usize)
                .map(|(_, option)| option)
                .collect()
        }
        _ => Vec::new(),
    };

    let multi_select = kind == FieldKind::List && flags.contains(FieldFlags::MULTI_SELECT);
    // A check box or radio button without an "on" state cannot be turned on.
    let stateless = matches!(kind, FieldKind::Checkbox | FieldKind::Radio) && on_value.is_none();
    let read_only = widget.is_readonly()?
        || flags.contains(FieldFlags::READ_ONLY)
        || multi_select
        || stateless
        || cut;
    let max_len = if kind == FieldKind::Text {
        object
            .get_dict_inheritable("MaxLen")?
            .and_then(|length| length.as_int().ok())
            .and_then(|length| u32::try_from(length).ok())
            .filter(|&length| length > 0)
            .map(|length| length.min(MAX_FIELD_VALUE_BYTES))
    } else {
        None
    };

    // The field the widget belongs to; a widget that is its own field is its own group.
    let head = field_head(&object)?;
    let group = if head.is_indirect()? {
        u32::try_from(head.as_indirect()?)
            .ok()
            .filter(|&number| number > 0)
            .unwrap_or(id)
    } else {
        id
    };

    Ok(Some(FormField {
        id: FieldId(id),
        group: FieldId(group),
        kind,
        rect,
        label,
        value,
        on_value,
        options,
        read_only,
        required: flags.contains(FieldFlags::REQUIRED),
        multiline,
        password: kind == FieldKind::Text && flags.contains(FieldFlags::PASSWORD),
        editable: kind == FieldKind::Combo && flags.contains(FieldFlags::EDIT),
        multi_select,
        max_len,
        has_script: has_script(&object)?,
    }))
}

/// What a check box or radio button is: the state its appearance shows (`/AS`), else the value
/// of its field, else off.
fn button_state(widget: &PdfObject) -> Result<String, EngineError> {
    for key in ["AS", "V"] {
        let Some(value) = widget.get_dict_inheritable(key)? else {
            continue;
        };
        let state = if value.is_name()? {
            Some(String::from_utf8_lossy(&value.as_name()?).into_owned())
        } else if value.is_string()? {
            Some(value.as_string()?)
        } else {
            None
        };
        if let Some(state) = state.filter(|state| !state.is_empty()) {
            return Ok(clean_field_text(&state, MAX_TEXT_BYTES as usize, false).0);
        }
    }
    Ok("Off".to_owned())
}

/// The state a check box or radio button is in when on: the name in its appearances other than
/// `Off`. `None` if it has none, or if the name is not text.
fn on_state(widget: &PdfObject) -> Result<Option<String>, EngineError> {
    let Some(normal) = widget
        .get_dict("AP")?
        .and_then(|appearances| appearances.get_dict("N").ok().flatten())
    else {
        return Ok(None);
    };
    if !normal.is_dict()? {
        return Ok(None);
    }
    for index in 0..normal.dict_len()? {
        let Some(key) = normal.get_dict_key(i32::try_from(index).unwrap_or(i32::MAX))? else {
            continue;
        };
        if !key.is_name()? {
            continue;
        }
        let name = key.as_name()?;
        if name != b"Off" {
            return Ok(String::from_utf8(name)
                .ok()
                .map(|name| clean_field_text(&name, MAX_TEXT_BYTES as usize, false).0)
                .filter(|name| !name.is_empty()));
        }
    }
    Ok(None)
}

/// The choices of a combo box or list box (`/Opt`: texts, or pairs of the value kept and the text
/// shown): as the file has them, and as the page gets them.
fn raw_options(widget: &PdfObject) -> Result<Vec<(String, FieldOption)>, EngineError> {
    let Some(options) = widget.get_dict_inheritable("Opt")? else {
        return Ok(Vec::new());
    };
    if !options.is_array()? {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for item in options.array_iter()? {
        // One more than may be listed, to tell that there were more.
        if found.len() > MAX_FIELD_OPTIONS as usize {
            break;
        }
        let Ok(item) = item else { continue };
        let (value, label) = if item.is_array()? {
            match (item.get_array(0)?, item.get_array(1)?) {
                (Some(value), Some(label)) if value.is_string()? && label.is_string()? => {
                    (value.as_string()?, label.as_string()?)
                }
                _ => continue,
            }
        } else if item.is_string()? {
            let text = item.as_string()?;
            (text.clone(), text)
        } else {
            continue;
        };
        let clean = |text: &str| clean_field_text(text, MAX_TEXT_BYTES as usize, false).0;
        found.push((
            value.clone(),
            FieldOption {
                value: clean(&value),
                label: clean(&label),
            },
        ));
    }
    Ok(found)
}

/// Whether the field has scripts, or is made of an action that is one: in the widget or any field
/// above it (`/AA` to format, check, calculate or react to keys; `/A` of a button). They are
/// never run, MuPDF having no JavaScript; the page says so.
fn has_script(widget: &PdfObject) -> Result<bool, EngineError> {
    let mut node = widget.try_clone()?;
    for _ in 0..MAX_FIELD_DEPTH {
        if let Some(triggers) = node.get_dict("AA")?
            && triggers.is_dict()?
        {
            for index in 0..triggers.dict_len()? {
                if let Some(action) =
                    triggers.get_dict_val(i32::try_from(index).unwrap_or(i32::MAX))?
                    && is_script(&action)?
                {
                    return Ok(true);
                }
            }
        }
        if let Some(action) = node.get_dict("A")?
            && is_script(&action)?
        {
            return Ok(true);
        }
        match node.get_dict("Parent")? {
            Some(parent) if parent.is_dict()? => node = parent,
            _ => break,
        }
    }
    Ok(false)
}

/// Whether `action` is a JavaScript action.
fn is_script(action: &PdfObject) -> Result<bool, EngineError> {
    if !action.is_dict()? {
        return Ok(false);
    }
    let named = action
        .get_dict("S")?
        .map(|name| name.as_name())
        .transpose()?
        .is_some_and(|name| name == b"JavaScript");
    Ok(named || action.get_dict("JS")?.is_some())
}

/// Puts check box or radio button `widget` in state `name` (`Off` or its "on" name), and with it
/// every widget of its field: the others of a radio group go off, and the field's value is the
/// state, as MuPDF's own forms do it. MuPDF writes a value as text; a name is what a button's is.
fn set_button_state(widget: &PdfObject, name: &str) -> Result<(), EngineError> {
    let mut head = field_head(widget)?;
    let mut budget = MAX_GROUP_WIDGETS;
    set_states(&head, name, 0, &mut budget)?;
    head.dict_put("V", PdfObject::new_name(name)?)?;
    Ok(())
}

/// The field a widget belongs to: the first of it and its parents that has a name (`/T`).
fn field_head(widget: &PdfObject) -> Result<PdfObject, EngineError> {
    let mut node = widget.try_clone()?;
    for _ in 0..MAX_FIELD_DEPTH {
        if node.get_dict("T")?.is_some() {
            return Ok(node);
        }
        match node.get_dict("Parent")? {
            Some(parent) if parent.is_dict()? => node = parent,
            _ => break,
        }
    }
    Ok(widget.try_clone()?)
}

/// Sets the appearance state of every widget under `node` to `name`, or to `Off` if its
/// appearances have no such state.
fn set_states(
    node: &PdfObject,
    name: &str,
    depth: usize,
    budget: &mut usize,
) -> Result<(), EngineError> {
    if depth > MAX_FIELD_DEPTH || *budget == 0 {
        return Err(EngineError::TooComplex);
    }
    *budget -= 1;
    if let Some(kids) = node.get_dict("Kids")?
        && kids.is_array()?
    {
        // A kid that cannot be read is left as it is.
        for kid in kids.array_iter()?.flatten() {
            set_states(&kid, name, depth + 1, budget)?;
        }
        return Ok(());
    }
    let has_state = match node
        .get_dict("AP")?
        .and_then(|appearances| appearances.get_dict("N").ok().flatten())
    {
        Some(normal) if normal.is_dict()? => normal.get_dict(name)?.is_some(),
        _ => false,
    };
    let mut terminal = node.try_clone()?;
    terminal.dict_put(
        "AS",
        PdfObject::new_name(if has_state { name } else { "Off" })?,
    )?;
    Ok(())
}
