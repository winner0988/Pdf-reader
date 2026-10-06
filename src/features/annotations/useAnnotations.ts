// What the user does with annotations (B2-07, docs/architecture/annotations.md): marking the
// selected text with the highlighter, putting a note where the context menu was opened, and
// changing a note. Each is an edit of the document, sent as any edit is; the tab's new state comes
// back on the open-events channel.

import { useRef, useState, type RefObject } from "react";

import type { EditingApi } from "@/features/thumbnails/api";
import type { DocumentViewHandle } from "@/features/viewer/DocumentView";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import {
  LIMITS,
  type DocumentId,
  type Edit,
  type HighlightColor,
  type PageAnnotation,
  type Point,
} from "@/ipc/generated/contract";

/** The note a dialog is open for: a new one at a place on a page, or what one already says. */
export type NoteTarget =
  | { kind: "add"; page: number; at: Point }
  | { kind: "edit"; page: number; annotation: PageAnnotation };

type Options = {
  doc?: DocumentId;
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
};

export function useAnnotations({ doc, editing, allowed, view, onProblem }: Options): AnnotationActions {
  const enabled = allowed && doc !== undefined && editing !== undefined;
  const [color, setColor] = useState<HighlightColor>("yellow");
  const [note, setNote] = useState<NoteTarget | null>(null);
  const menuAt = useRef<{ x: number; y: number } | null>(null);

  const edit = (change: Edit) => {
    if (!enabled) return;
    editing.applyEdit(doc, change).catch((error: unknown) => {
      // Too many unsaved changes to keep (B2-13): saving makes room.
      onProblem(errorCodeOf(error) === "limitExceeded" ? strings.pages.saveFirst : strings.annotations.failed);
    });
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
      if (at) setNote({ kind: "add", page: at.page, at: at.point });
      else onProblem(strings.annotations.notOnPage);
    },
    editNote(page, annotation) {
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
  };
}
