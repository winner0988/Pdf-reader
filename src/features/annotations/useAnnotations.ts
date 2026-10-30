// What the user does with annotations (B2-07, docs/architecture/annotations.md): marking the
// selected text with the highlighter, putting a note where the context menu was opened, changing a
// note, and (B2-30) putting down and changing text boxes. Each is an edit of the document, sent as
// any edit is; the tab's new state comes back on the open-events channel.

import { useEffect, useRef, useState, type RefObject } from "react";

import type { AnnotationsApi } from "@/features/annotations/source";
import type { TextBoxDraft, TextBoxStart } from "@/features/annotations/TextBoxDialog";
import { textBoxRectAt } from "@/features/annotations/tools";
import type { PageSize } from "@/features/shell/model";
import type { EditingApi } from "@/features/thumbnails/api";
import type { DocumentViewHandle } from "@/features/viewer/DocumentView";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import {
  LIMITS,
  type DocumentId,
  type Edit,
  type FontInfo,
  type HighlightColor,
  type PageAnnotation,
  type Point,
  type TextBoxStyle,
} from "@/ipc/generated/contract";

/** The note a dialog is open for: a new one at a place on a page, or what one already says. */
export type NoteTarget =
  | { kind: "add"; page: number; at: Point }
  | { kind: "edit"; page: number; annotation: PageAnnotation };

/** The text box a dialog is open for: a new one at a place on a page, or one already there. */
export type TextBoxTarget =
  | { kind: "add"; page: number; at: Point; fonts: FontInfo[] }
  | { kind: "edit"; page: number; annotation: PageAnnotation; fonts: FontInfo[]; start: TextBoxStart };

type Options = {
  doc?: DocumentId;
  /** Where fonts, prepared words and what a text box says come from; without it there are no text boxes. */
  api?: AnnotationsApi;
  /** The size of page `index`, to put a text box inside it. */
  pageSizeOf?: (index: number) => PageSize | undefined;
  editing?: EditingApi;
  /** The document's author allows annotating (`/P` bit 6, MVP-19). */
  allowed: boolean;
  view: RefObject<DocumentViewHandle | null>;
  /** Says what went wrong, in the status bar. */
  onProblem: (text: string) => void;
};

export type AnnotationActions = {
  /** Annotations can be added, changed and removed. */
  enabled: boolean;
  /** The color the highlighter used last (yellow at first). */
  color: HighlightColor;
  /** Changes the document; without it (the author does not allow it) annotations are only shown. */
  edit: ((edit: Edit) => void) | undefined;
  /** Marks the selected text; in `color`, or the last one used. */
  highlight(color?: HighlightColor): void;
  /** Remembers where the context menu opened, for `newNote`. */
  contextMenuAt(clientX: number, clientY: number): void;
  /** Opens the dialog for a new note where the context menu was opened. */
  newNote(): void;
  /** Opens the dialog for what the note `annotation` of `page` says. */
  editNote(page: number, annotation: PageAnnotation): void;
  note: NoteTarget | null;
  saveNote(text: string): void;
  closeNote(): void;
  /** Text boxes can be put down and changed (the main process has what they need). */
  textBoxes: boolean;
  /** Opens the dialog for a new text box at `at` on `page` (page space). */
  newTextBox(page: number, at: Point): void;
  /** Opens the dialog for a new text box where the context menu was opened. */
  newTextBoxAtMenu(): void;
  /** Opens the dialog for what the text box `annotation` of `page` says. */
  editTextBox(page: number, annotation: PageAnnotation): void;
  textBox: TextBoxTarget | null;
  /** The family the last text box was set in, to start the next with. */
  lastFamily: string | undefined;
  /** Makes the words and puts them in a box; says what went wrong (for the dialog), or `null` when done. */
  saveTextBox(draft: TextBoxDraft): Promise<string | null>;
  closeTextBox(): void;
};

/** Whether two styles are the same. */
function sameStyle(one: TextBoxStyle, other: TextBoxStyle): boolean {
  return (
    one.size === other.size &&
    one.align === other.align &&
    one.color.r === other.color.r &&
    one.color.g === other.color.g &&
    one.color.b === other.color.b
  );
}

export function useAnnotations({
  doc,
  api,
  pageSizeOf,
  editing,
  allowed,
  view,
  onProblem,
}: Options): AnnotationActions {
  const enabled = allowed && doc !== undefined && editing !== undefined;
  const [color, setColor] = useState<HighlightColor>("yellow");
  const [note, setNote] = useState<NoteTarget | null>(null);
  const menuAt = useRef<{ x: number; y: number } | null>(null);
  const [textBox, setTextBox] = useState<TextBoxTarget | null>(null);
  const [lastFamily, setLastFamily] = useState<string | undefined>(undefined);
  const textBoxes = enabled && api?.getFonts !== undefined && api.prepareTextBox !== undefined;

  // Where the focus was when a dialog opened: an annotation's outline, say, or a button of its
  // toolbar. A dialog closes by going away, which leaves the focus nowhere; it goes back there,
  // unless that is gone or the user has moved on (#402).
  const focusBefore = useRef<Element | null>(null);
  const opening = () => {
    focusBefore.current = document.activeElement;
  };
  const dialogOpen = note !== null || textBox !== null;
  useEffect(() => {
    if (dialogOpen) return;
    const before = focusBefore.current;
    focusBefore.current = null;
    const nowhere = document.activeElement === null || document.activeElement === document.body;
    if (before instanceof HTMLElement && before.isConnected && nowhere) before.focus();
  }, [dialogOpen]);

  const edit = (change: Edit) => {
    if (!enabled) return;
    editing.applyEdit(doc, change).catch((error: unknown) => {
      // Too many unsaved changes to keep (B2-13): saving makes room.
      onProblem(errorCodeOf(error) === "limitExceeded" ? strings.pages.saveFirst : strings.annotations.failed);
    });
  };

  const openNewTextBox = (page: number, at: Point) => {
    if (!textBoxes || !api?.getFonts) return;
    // The list is read again every time: a font installed a moment ago is in it.
    api.getFonts().then(
      (fonts) => {
        opening();
        setTextBox({ kind: "add", page, at, fonts });
      },
      () => onProblem(strings.annotations.fontsFailed),
    );
  };

  return {
    enabled,
    color,
    edit: enabled ? edit : undefined,
    highlight(use = color) {
      if (!enabled) return;
      void view.current?.highlightMarks().then((marks) => {
        if (marks.length === 0) return;
        const quads = marks.reduce((total, mark) => total + mark.quads.length, 0);
        if (marks.length > LIMITS.maxHighlightPages || quads > LIMITS.maxAnnotationQuads) {
          onProblem(strings.annotations.tooMuch);
          return;
        }
        setColor(use);
        edit({ kind: "addHighlight", marks, color: use });
      });
    },
    contextMenuAt(clientX, clientY) {
      menuAt.current = { x: clientX, y: clientY };
    },
    newNote() {
      const at = menuAt.current && view.current?.pageAt(menuAt.current.x, menuAt.current.y);
      if (at) {
        opening();
        setNote({ kind: "add", page: at.page, at: at.point });
      } else onProblem(strings.annotations.notOnPage);
    },
    editNote(page, annotation) {
      opening();
      setNote({ kind: "edit", page, annotation });
    },
    note,
    saveNote(text) {
      const target = note;
      setNote(null);
      if (!target) return;
      edit(
        target.kind === "add"
          ? { kind: "addNote", page: target.page, at: target.at, text }
          : { kind: "setNoteText", page: target.page, annotation: target.annotation.id, text },
      );
    },
    closeNote() {
      setNote(null);
    },
    textBoxes,
    textBox,
    lastFamily,
    newTextBox: openNewTextBox,
    newTextBoxAtMenu() {
      const at = menuAt.current && view.current?.pageAt(menuAt.current.x, menuAt.current.y);
      if (!at) onProblem(strings.annotations.notOnPageForBox);
      else openNewTextBox(at.page, at.point);
    },
    editTextBox(page, annotation) {
      if (!textBoxes || !api?.getFonts || !api.getTextBox || doc === undefined) return;
      Promise.all([api.getFonts(), api.getTextBox(doc, page, annotation.id)]).then(
        ([fonts, info]) => {
          opening();
          setTextBox({
            kind: "edit",
            page,
            annotation,
            fonts,
            start: { text: info.text, font: info.font, style: info.style, overflow: info.overflow },
          });
        },
        () => onProblem(strings.annotations.fontsFailed),
      );
    },
    async saveTextBox(draft) {
      const t = strings.annotations.textBoxDialog;
      const target = textBox;
      if (!target || !textBoxes || !api?.prepareTextBox || doc === undefined) return t.failed;
      // Words and a font are prepared by the main process; a box is made of what comes back.
      const prepare = async (font: FontInfo) => {
        const prepared = await api.prepareTextBox!(doc, draft.text, font.id);
        if (prepared.viewOnly) onProblem(strings.annotations.textBoxViewOnly(font.name));
        return prepared.model;
      };
      try {
        if (target.kind === "add") {
          const size = pageSizeOf?.(target.page);
          if (!draft.font || !size) return t.failed;
          const model = await prepare(draft.font);
          edit({ kind: "addTextBox", page: target.page, rect: textBoxRectAt(target.at, size), model, style: draft.style });
        } else {
          const own = target.start;
          const fontChanged = draft.font !== null && draft.font.name !== own.font;
          if (draft.text !== own.text || fontChanged) {
            // Other words or another font need a font: the one the box has may be gone.
            if (!draft.font) return t.needFont;
            const model = await prepare(draft.font);
            edit({ kind: "setTextBox", page: target.page, annotation: target.annotation.id, model, style: draft.style });
          } else if (!sameStyle(draft.style, own.style)) {
            edit({ kind: "setTextBox", page: target.page, annotation: target.annotation.id, model: null, style: draft.style });
          }
        }
      } catch (error) {
        const code = errorCodeOf(error);
        if (code === "limitExceeded") return strings.pages.saveFirst;
        return (code && strings.error.messages[code]) || t.failed;
      }
      if (draft.font) setLastFamily(draft.font.family);
      setTextBox(null);
      return null;
    },
    closeTextBox() {
      setTextBox(null);
    },
  };
}
