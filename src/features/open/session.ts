// What the window shows, derived from open events (MVP-06). Pure, so it is tested without IPC.

import type { ShellDocument, ShellState } from "@/features/shell/model";
import type { DocumentId, DocumentInfo, ErrorCode, OpenEvent } from "@/ipc/generated/contract";

/** "Only the first of several dropped files was opened" (docs/ux/screen-map.md, dropMultiple). */
export type OpenNotice = { kind: "dropMultiple"; displayName: string };

export type OpenSession = {
  shell: ShellState;
  /** The open document's id, needed to close it. */
  doc: DocumentId | null;
  /** Files are being dragged over the window. */
  dragActive: boolean;
  notice: OpenNotice | null;
};

export type SessionAction =
  | { type: "event"; event: OpenEvent }
  | { type: "closed" }
  | { type: "failed"; code: ErrorCode }
  | { type: "dismissNotice" };

export const initialSession: OpenSession = {
  shell: { kind: "empty" },
  doc: null,
  dragActive: false,
  notice: null,
};

export function toShellDocument(info: DocumentInfo): ShellDocument {
  return {
    displayName: info.displayName,
    pages: info.pages,
    outline: [], // MVP-09
    findings: info.security.findings,
  };
}

function dropNotice(ignoredFiles: number, displayName: string): OpenNotice | null {
  return ignoredFiles > 0 ? { kind: "dropMultiple", displayName } : null;
}

export function reduceSession(session: OpenSession, action: SessionAction): OpenSession {
  switch (action.type) {
    case "closed":
      return { ...session, shell: { kind: "empty" }, doc: null, notice: null };
    case "failed":
      return { ...session, shell: { kind: "error", code: action.code }, doc: null };
    case "dismissNotice":
      return { ...session, notice: null };
    case "event":
      break;
  }
  const event = action.event;
  switch (event.kind) {
    case "dragHover":
      return { ...session, dragActive: event.active };
    case "opening":
      return {
        ...session,
        shell: { kind: "loading", displayName: event.displayName },
        doc: null,
        dragActive: false,
        notice: null,
      };
    case "opened":
      return {
        ...session,
        shell: { kind: "open", document: toShellDocument(event.info) },
        doc: event.info.doc,
        notice: dropNotice(event.ignoredFiles, event.info.displayName),
      };
    case "failed":
      return {
        ...session,
        shell: { kind: "error", code: event.error.code, displayName: event.displayName },
        doc: null,
        notice: dropNotice(event.ignoredFiles, event.displayName),
      };
  }
}
