// What the user does with annotations (B2-07, docs/architecture/annotations.md): marking the
// selected text with the highlighter, putting a note where the context menu was opened, changing a
// note, and (B2-30) putting down and changing text boxes. Each is an edit of the document, sent as
// any edit is; the tab's new state comes back on the open-events channel.

import { useEffect, useRef, useState, type RefObject } from "react";

import type { InlineTextDraft, InlineTextStart } from "@/features/annotations/InlineTextEditor";
import type { AnnotationsApi } from "@/features/annotations/source";
import { newInlineTextBoxRect, sameRect, sameStyle } from "@/features/annotations/textBox";
import { until, type FieldEdits } from "@/features/forms/edits";
import type { PageSize } from "@/features/shell/model";
import type { EditingApi } from "@/features/thumbnails/api";
import type { DocumentViewHandle } from "@/features/viewer/DocumentView";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n";
import {
  LIMITS,
  type DocumentId,
  type Edit,
  type FontInfo,
  type HighlightColor,
  type PageAnnotation,
  type Point,
  type Rect,
  type TextMarkKind,
} from "@/ipc/generated/contract";

/** The note a dialog is open for: a new one at a place on a page, or what one already says. */
export type NoteTarget =
  | { kind: "add"; page: number; at: Point }
  | { kind: "edit"; page: number; annotation: PageAnnotation };

/** The text box being typed inline on a page: a new one or one already there (#719). */
export type TextBoxTarget =
  | { kind: "add"; page: number; rect: Rect; fonts: FontInfo[] }
  | {
      kind: "edit";
      page: number;
      annotation: PageAnnotation;
      fonts: FontInfo[];
      start: InlineTextStart;
      clickClient?: { x: number; y: number };
    };

type Options = {
  doc?: DocumentId;
  /** Where fonts, prepared words and what a text box says come from; without it there are no text boxes. */
  api?: AnnotationsApi;
  /** The size of page `index`, to put a text box inside it. */
  pageSizeOf?: (index: number) => PageSize | undefined;
  editing?: EditingApi;
  /** Shared edit queue so Ctrl+S / close waits for an inline text box commit and its new doc id. */
  edits?: FieldEdits;
  /** The latest announced document id for the tab. */
  latest?: () => { doc: DocumentId; unsaved: boolean } | undefined;
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
  /** The last color used for each mark kind. */
  markColor(kind: TextMarkKind): HighlightColor;
  /** Changes the document; without it (the author does not allow it) annotations are only shown. */
  edit: ((edit: Edit) => void) | undefined;
  /** Marks the selected text; in `color`, or the last one used for that kind. */
  highlight(color?: HighlightColor, markKind?: TextMarkKind): void;
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
  /** Starts typing a new text box inline at `at` on `page` (page space). */
  newTextBox(page: number, at: Point, options?: { centered?: boolean }): void;
  /** Starts typing a new text box inline where the context menu was opened. */
  newTextBoxAtMenu(): void;
  /** Starts editing an existing text box inline on `page`. */
  editTextBox(page: number, annotation: PageAnnotation, clickClient?: { x: number; y: number }): void;
  textBox: TextBoxTarget | null;
  /** The family the last text box was set in, to start the next with. */
  lastFamily: string | undefined;
  /** Makes the words and commits them in one edit; says what went wrong, or `null` when done. */
  saveTextBox(draft: InlineTextDraft, options?: { restoreFocus?: boolean }): Promise<string | null>;
  /** Cancels an empty new text box without sending an edit. */
  closeTextBox(options?: { restoreFocus?: boolean }): void;
};

const DEFAULT_MARK_COLORS: Record<TextMarkKind, HighlightColor> = {
  highlight: "yellow",
  underline: "blue",
  strikeOut: "pink",
  squiggly: "green",
};

export function useAnnotations({
  doc,
  api,
  pageSizeOf,
  editing,
  edits,
  latest,
  allowed,
  view,
  onProblem,
}: Options): AnnotationActions {
  const enabled = allowed && doc !== undefined && editing !== undefined;
  const [markColors, setMarkColors] = useState<Record<TextMarkKind, HighlightColor>>(DEFAULT_MARK_COLORS);
  const [note, setNote] = useState<NoteTarget | null>(null);
  const menuAt = useRef<{ x: number; y: number } | null>(null);
  const [textBox, setTextBox] = useState<TextBoxTarget | null>(null);
  const [lastFamily, setLastFamily] = useState<string | undefined>(undefined);
  const textBoxes = enabled && api?.getFonts !== undefined && api.prepareTextBox !== undefined;

  // Where the focus was when a dialog or inline editor opened: an annotation's outline, say, or a
  // button of its toolbar. Closing with Esc leaves focus nowhere; it goes back there (#402).
  const focusBefore = useRef<Element | null>(null);
  const opening = () => {
    focusBefore.current = document.activeElement;
  };
  const editorOpen = note !== null || textBox !== null;
  useEffect(() => {
    if (editorOpen) return;
    const before = focusBefore.current;
    focusBefore.current = null;
    const nowhere = document.activeElement === null || document.activeElement === document.body;
    if (before instanceof HTMLElement && before.isConnected && nowhere) before.focus();
  }, [editorOpen]);

  const currentDoc = () => latest?.()?.doc ?? doc;

  const edit = (change: Edit) => {
    const activeDoc = currentDoc();
    if (!enabled || activeDoc === undefined) return;
    editing.applyEdit(activeDoc, change).catch((error: unknown) => {
      // Too many unsaved changes to keep (B2-13): saving makes room.
      onProblem(errorCodeOf(error) === "limitExceeded" ? strings.pages.saveFirst : strings.annotations.failed);
    });
  };

  /** Sends an edit through `edits` (if provided) and waits for the new doc id announcement. */
  const applySettledEdit = async (change: Edit): Promise<void> => {
    if (!enabled) throw new Error("editing disabled");
    const run = async () => {
      const before = currentDoc();
      if (before === undefined) throw new Error("no document");
      await editing.applyEdit(before, change);
      if (latest) await until(() => latest()?.doc !== before);
    };
    if (edits) await edits.run(run);
    else await run();
  };

  const openNewTextBox = (page: number, at: Point, options?: { centered?: boolean }) => {
    if (!textBoxes || !api?.getFonts) return;
    const size = pageSizeOf?.(page);
    if (!size) return;
    // The list is read again every time: a font installed a moment ago is in it.
    api.getFonts().then(
      (fonts) => {
        opening();
        const rect = newInlineTextBoxRect(at, size, undefined, undefined, options?.centered === true);
        setTextBox({ kind: "add", page, rect, fonts });
      },
      () => onProblem(strings.annotations.fontsFailed),
    );
  };

  return {
    enabled,
    color: markColors.highlight,
    markColor(kind: TextMarkKind) {
      return markColors[kind];
    },
    edit: enabled ? edit : undefined,
    highlight(useColor, markKind = "highlight") {
      if (!enabled) return;
      const use = useColor ?? markColors[markKind];
      void view.current?.highlightMarks().then((marks) => {
        if (marks.length === 0) return;
        const quads = marks.reduce((total, mark) => total + mark.quads.length, 0);
        if (marks.length > LIMITS.maxHighlightPages || quads > LIMITS.maxAnnotationQuads) {
          onProblem(strings.annotations.tooMuch);
          return;
        }
        setMarkColors((prev) => ({ ...prev, [markKind]: use }));
        edit({ kind: "addHighlight", markKind, marks, color: use });
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
    editTextBox(page, annotation, clickClient) {
      const activeDoc = currentDoc();
      if (!textBoxes || !api?.getFonts || !api.getTextBox || activeDoc === undefined) return;
      Promise.all([api.getFonts(), api.getTextBox(activeDoc, page, annotation.id)]).then(
        ([fonts, info]) => {
          opening();
          setTextBox({
            kind: "edit",
            page,
            annotation,
            fonts,
            start: {
              text: info.text,
              font: info.font,
              style: info.style,
              overflow: info.overflow,
              viewOnly: info.viewOnly,
              lineStep: info.lineStep,
            },
            clickClient,
          });
        },
        () => onProblem(strings.annotations.fontsFailed),
      );
    },
    async saveTextBox(draft, options) {
      const e = strings.annotations.textBoxEditor;
      const target = textBox;
      const activeDoc = currentDoc();
      if (!target || !textBoxes || !api?.prepareTextBox || activeDoc === undefined) return e.failed;
      if (options?.restoreFocus === false) focusBefore.current = null;
      // Words and a font are prepared by the main process; a box is made of what comes back.
      const prepare = async (font: FontInfo) => {
        const prepared = await api.prepareTextBox!(activeDoc, draft.text, font.id);
        if (prepared.viewOnly) onProblem(strings.annotations.textBoxViewOnly(font.name));
        return prepared.model;
      };
      try {
        if (target.kind === "add") {
          if (draft.text === "") {
            setTextBox(null);
            return null;
          }
          if (!draft.font) return e.failed;
          const model = await prepare(draft.font);
          await applySettledEdit({
            kind: "addTextBox",
            page: target.page,
            rect: draft.rect,
            model,
            style: draft.style,
          });
        } else if (draft.text === "") {
          // Existing text box cleared to empty: delete in one undoable edit.
          await applySettledEdit({
            kind: "deleteAnnotation",
            page: target.page,
            annotation: target.annotation.id,
          });
        } else {
          const own = target.start;
          const fontChanged = draft.font !== null && draft.font.name !== own.font;
          const rectChanged = !sameRect(draft.rect, target.annotation.rect);
          const rectField = rectChanged ? { rect: draft.rect } : {};
          if (draft.text !== own.text || fontChanged) {
            // Other words or another font need a font: the one the box has may be gone.
            if (!draft.font) return e.needFont;
            const model = await prepare(draft.font);
            await applySettledEdit({
              kind: "setTextBox",
              page: target.page,
              annotation: target.annotation.id,
              model,
              style: draft.style,
              ...rectField,
            });
          } else if (!sameStyle(draft.style, own.style) || rectChanged) {
            await applySettledEdit({
              kind: "setTextBox",
              page: target.page,
              annotation: target.annotation.id,
              model: null,
              style: draft.style,
              ...rectField,
            });
          }
        }
      } catch (error) {
        const code = errorCodeOf(error);
        if (code === "limitExceeded") return strings.pages.saveFirst;
        return (code && strings.error.messages[code]) || e.failed;
      }
      if (draft.font) setLastFamily(draft.font.family);
      setTextBox(null);
      return null;
    },
    closeTextBox(options) {
      if (options?.restoreFocus === false) focusBefore.current = null;
      setTextBox(null);
    },
  };
}
