// A page's form fields over it (B2-09, docs/architecture/forms.md): a text box, check box, radio
// button, drop-down list or list box for each, where the field is. They sit next to the page, not
// inside it (the page is an image to assistive technology, which would hide them). A value is
// sent to the document when the user is done with it (leaves a text box, or picks); it is shown as
// the user made it until the document has it, so that nothing jumps back, and the focus stays.

import { useEffect, useId, useRef, useState, type ChangeEvent, type CSSProperties, type KeyboardEvent } from "react";

import type { FormSource } from "@/features/forms/source";
import type { PageSize, Rotation } from "@/features/shell/model";
import { rectToBox, type PageBox } from "@/features/viewer/layout";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, Edit, FormField } from "@/ipc/generated/contract";

const t = strings.forms;

/** What the user made of a field, until the document has it (or it could not be made). */
function useDraft(server: string) {
  const [draft, setDraft] = useState<{ value: string; basedOn: string } | null>(null);
  // A value from the document ends the draft: it became what the draft was, or was replaced.
  const shown = draft !== null && draft.basedOn === server ? draft.value : server;
  return {
    shown,
    edit: (value: string) => setDraft({ value, basedOn: server }),
    clear: () => setDraft(null),
  };
}

type PageFieldsProps = {
  source: FormSource;
  doc: DocumentId;
  index: number;
  page: PageSize;
  rotation: Rotation;
  box: PageBox;
  left: number;
  delayMs: number;
  /** The author allows filling in forms (MVP-19): without it the fields are only shown. */
  allowed: boolean;
  /** Changes the document; rejects if it could not. */
  onEdit: (edit: Edit) => Promise<void>;
  /** The user entered a field that has scripts: they are not run. */
  onScript: () => void;
};

export function PageFields({
  source,
  doc,
  index,
  page,
  rotation,
  box,
  left,
  delayMs,
  allowed,
  onEdit,
  onScript,
}: PageFieldsProps) {
  // The fields of the last document that answered: an edit gives the document a new id, and the
  // box the user is in must stay where it is until the new list arrives.
  const [fields, setFields] = useState<FormField[]>([]);

  useEffect(() => {
    let current = true;
    const load = () =>
      source.fields(doc, index).then(
        (answer) => {
          if (current) setFields(answer);
        },
        // No fields is better than an error for what is only an aid: the page is still there.
        () => {},
      );
    // Like renders: pages that only flash by during a fast scroll are not asked.
    const timer = delayMs > 0 ? window.setTimeout(load, delayMs) : undefined;
    if (timer === undefined) void load();
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [source, doc, index, delayMs]);

  if (fields.length === 0) return null;
  return (
    <div
      data-page-fields={index + 1}
      className="pointer-events-none absolute"
      style={{ top: box.top, left, width: box.width, height: box.height }}
    >
      {fields.map((field) => {
        const area = rectToBox(field.rect, page, rotation, box);
        // About two thirds of the field's height, as a form's own text is, within reason.
        const shown = area.height / Math.max(field.rect.y1 - field.rect.y0, 1);
        const fontSize = Math.max(9, Math.min(area.height * 0.66, 14 * shown));
        return (
          <FieldControl
            key={field.id}
            field={field}
            style={{ left: area.left, top: area.top, width: area.width, height: area.height, fontSize }}
            allowed={allowed}
            commit={(value) => onEdit({ kind: "setFieldValue", page: index, field: field.id, value })}
            onScript={onScript}
          />
        );
      })}
    </div>
  );
}

type ControlProps = {
  field: FormField;
  style: CSSProperties;
  allowed: boolean;
  /** Gives the field `value` in the document; rejects if it could not. */
  commit: (value: string) => Promise<void>;
  onScript: () => void;
};

const BASE =
  "pointer-events-auto absolute box-border border bg-sky-50/90 text-neutral-900 outline-offset-1 " +
  "focus-visible:outline-2 focus-visible:outline-primary disabled:opacity-60 read-only:bg-neutral-100/80";

/** What every control says about its field, to the screen reader and the mouse. */
function describe(field: FormField, allowed: boolean) {
  return {
    "aria-label": field.label ?? undefined,
    "aria-required": field.required || undefined,
    title: !allowed ? t.notAllowed : field.readOnly ? t.readOnly : (field.label ?? undefined),
    "data-field": field.kind,
    "data-script": field.hasScript || undefined,
  };
}

function FieldControl(props: ControlProps) {
  switch (props.field.kind) {
    case "text":
      return <TextControl {...props} />;
    case "checkbox":
    case "radio":
      return <ButtonControl {...props} />;
    case "combo":
    case "list":
      return <ChoiceControl {...props} />;
  }
}

/** A text field: sent when the user leaves it, or presses Enter (Ctrl+Enter in several lines). */
function TextControl({ field, style, allowed, commit, onScript }: ControlProps) {
  const { shown, edit, clear } = useDraft(field.value);
  // Escape gives the draft up: leaving the box then sends nothing.
  const discarding = useRef(false);
  const missing = field.required && shown === "";
  const attributes = {
    ...describe(field, allowed),
    value: shown,
    readOnly: field.readOnly || !allowed,
    maxLength: field.maxLen ?? undefined,
    autoComplete: "off",
    spellCheck: false,
    "aria-invalid": missing || undefined,
    className: `${BASE} ${missing ? "border-red-500" : "border-sky-500/60"} px-1`,
    style,
  };
  const onChange = (event: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => edit(event.target.value);
  const onFocus = () => field.hasScript && onScript();
  const onBlur = () => {
    if (discarding.current) {
      discarding.current = false;
      clear();
    } else if (shown !== field.value) {
      commit(shown).catch(clear);
    }
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    if (event.key === "Escape") {
      discarding.current = true;
      event.currentTarget.blur();
    } else if (event.key === "Enter" && (!field.multiline || event.ctrlKey)) {
      event.currentTarget.blur();
    }
  };
  return field.multiline ? (
    <textarea
      {...attributes}
      className={`${attributes.className} resize-none py-0.5 leading-tight`}
      onChange={onChange}
      onFocus={onFocus}
      onBlur={onBlur}
      onKeyDown={onKeyDown}
    />
  ) : (
    <input
      {...attributes}
      type={field.password ? "password" : "text"}
      onChange={onChange}
      onFocus={onFocus}
      onBlur={onBlur}
      onKeyDown={onKeyDown}
    />
  );
}

/** A check box or a radio button: sent at once. */
function ButtonControl({ field, style, allowed, commit, onScript }: ControlProps) {
  const { shown, edit, clear } = useDraft(field.value);
  const on = field.onValue ?? "";
  const checked = on !== "" && shown === on;
  const named = describe(field, allowed);
  // Every button of a group has the group's name: each says which choice it is.
  if (field.kind === "radio" && on !== "") {
    named["aria-label"] = field.label ? t.choice(field.label, on) : on;
  }
  return (
    <input
      {...named}
      type={field.kind === "radio" ? "radio" : "checkbox"}
      // The radio buttons of a group are one stop of Tab, and the arrow keys move between them.
      name={field.kind === "radio" ? `field-${field.group}` : undefined}
      checked={checked}
      disabled={field.readOnly || !allowed}
      className={`${BASE} border-sky-500/60 accent-sky-600`}
      style={{ ...style, padding: 0 }}
      onFocus={() => field.hasScript && onScript()}
      onChange={() => {
        // A radio button is only turned on; a check box is turned either way.
        const next = field.kind === "radio" || !checked ? on : "Off";
        edit(next);
        commit(next).catch(clear);
      }}
    />
  );
}

/** A drop-down list (which may also be typed in) or a list box: sent when the user picks. */
function ChoiceControl({ field, style, allowed, commit, onScript }: ControlProps) {
  const { shown, edit, clear } = useDraft(field.value);
  const listId = useId();
  const locked = field.readOnly || !allowed;
  const send = (value: string) => {
    edit(value);
    commit(value).catch(clear);
  };
  if (field.kind === "combo" && field.editable) {
    return (
      <>
        <input
          {...describe(field, allowed)}
          type="text"
          list={listId}
          value={shown}
          readOnly={locked}
          autoComplete="off"
          spellCheck={false}
          className={`${BASE} border-sky-500/60 px-1`}
          style={style}
          onFocus={() => field.hasScript && onScript()}
          onChange={(event) => edit(event.target.value)}
          onBlur={() => shown !== field.value && send(shown)}
          onKeyDown={(event) => event.key === "Enter" && event.currentTarget.blur()}
        />
        <datalist id={listId}>
          {field.options.map((option) => (
            <option key={option.value} value={option.value} label={option.label} />
          ))}
        </datalist>
      </>
    );
  }
  return (
    <select
      {...describe(field, allowed)}
      value={shown}
      size={field.kind === "list" ? Math.max(2, Math.min(field.options.length, 8)) : undefined}
      disabled={locked}
      className={`${BASE} border-sky-500/60 px-0.5`}
      style={style}
      onFocus={() => field.hasScript && onScript()}
      onChange={(event) => send(event.target.value)}
    >
      {!field.options.some((option) => option.value === shown) && <option value={shown}>{shown}</option>}
      {field.options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}
