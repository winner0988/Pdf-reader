// The window's tabs (MVP-14, ADR 0012), derived from open events and the user's tab actions.
// Pure, so it is tested without IPC.

import type { ShellDocument, ShellState } from "@/features/shell/model";
import type { DocumentId, DocumentInfo, ErrorCode, OpenEvent, TabId } from "@/ipc/generated/contract";

export type TabContent =
  | { kind: "loading" }
  | { kind: "open"; document: ShellDocument; hasOutline: boolean }
  /** An encrypted file waits for its password (MVP-16); `wrong` after one that did not open it. */
  | { kind: "password"; wrong: boolean }
  | { kind: "error"; code: ErrorCode };

export type Tab = { tab: TabId; displayName: string; content: TabContent };

export type TabsNotice =
  /** Files dropped or picked beyond the tab limit were not opened. */
  | { kind: "tabLimit"; ignoredFiles: number }
  /** A command to the main process failed outright (e.g. the open dialog could not be shown). */
  | { kind: "failed"; code: ErrorCode };

export type TabsState = {
  /** In the order the tabs were added. */
  tabs: Tab[];
  active: TabId | null;
  /** Tabs the user closed. Tab ids are never reused, so events still on their way for these are ignored. */
  closed: TabId[];
  /** Files are being dragged over the window. */
  dragActive: boolean;
  notice: TabsNotice | null;
};

export type TabsAction =
  | { type: "event"; event: OpenEvent }
  | { type: "activate"; tab: TabId }
  | { type: "step"; by: 1 | -1 }
  | { type: "closed"; tab: TabId }
  | { type: "failed"; code: ErrorCode }
  | { type: "dismissNotice" };

export const initialTabs: TabsState = { tabs: [], active: null, closed: [], dragActive: false, notice: null };

export function toShellDocument(info: DocumentInfo): ShellDocument {
  return {
    doc: info.doc,
    displayName: info.displayName,
    pages: info.pages,
    findings: info.security.findings,
    scanComplete: info.security.scanComplete,
  };
}

/** Element ids that tie a tab to its panel (aria-controls, aria-labelledby). */
export const tabElementId = (tab: TabId) => `tab-${tab}`;
export const tabPanelId = (tab: TabId) => `tabpanel-${tab}`;

/** What a tab's pane shows. */
export function shellState(tab: Tab): ShellState {
  switch (tab.content.kind) {
    case "loading":
      return { kind: "loading", displayName: tab.displayName };
    case "open":
      return { kind: "open", document: tab.content.document };
    case "password":
      return { kind: "password", displayName: tab.displayName, wrong: tab.content.wrong };
    case "error":
      return { kind: "error", code: tab.content.code, displayName: tab.displayName };
  }
}

/** The document a tab shows, once it has opened. */
export function docOf(tab: Tab): DocumentId | null {
  return tab.content.kind === "open" ? (tab.content.document.doc ?? null) : null;
}

/**
 * Adds `tab` or replaces the tab with its id. A new tab is shown at once when `show` is set;
 * a page that was reloaded hears about the existing tabs again and shows the first one.
 */
function upsert(state: TabsState, tab: Tab, show: boolean): TabsState {
  if (state.closed.includes(tab.tab)) return state;
  const index = state.tabs.findIndex((existing) => existing.tab === tab.tab);
  const tabs = index === -1 ? [...state.tabs, tab] : state.tabs.map((existing, i) => (i === index ? tab : existing));
  const active = (index === -1 && show) || state.active === null ? tab.tab : state.active;
  return { ...state, tabs, active };
}

export function reduceTabs(state: TabsState, action: TabsAction): TabsState {
  switch (action.type) {
    case "activate":
      return state.tabs.some((tab) => tab.tab === action.tab) ? { ...state, active: action.tab } : state;
    case "step": {
      if (state.tabs.length === 0) return state;
      const index = state.tabs.findIndex((tab) => tab.tab === state.active);
      const next = (index + action.by + state.tabs.length) % state.tabs.length;
      return { ...state, active: state.tabs[next]?.tab ?? state.active };
    }
    case "closed": {
      const index = state.tabs.findIndex((tab) => tab.tab === action.tab);
      if (index === -1) return state;
      const tabs = state.tabs.filter((tab) => tab.tab !== action.tab);
      // Like a browser: the tab on the right takes the closed one's place, else the one on the left.
      const active = state.active === action.tab ? (tabs[index] ?? tabs[index - 1] ?? null)?.tab ?? null : state.active;
      return { ...state, tabs, active, closed: [...state.closed, action.tab] };
    }
    case "failed":
      return { ...state, notice: { kind: "failed", code: action.code } };
    case "dismissNotice":
      return { ...state, notice: null };
    case "event":
      break;
  }
  const event = action.event;
  switch (event.kind) {
    case "dragHover":
      return { ...state, dragActive: event.active };
    case "opening":
      return {
        ...upsert(state, { tab: event.tab, displayName: event.displayName, content: { kind: "loading" } }, true),
        dragActive: false,
      };
    case "opened":
      return upsert(
        state,
        {
          tab: event.tab,
          displayName: event.info.displayName,
          content: { kind: "open", document: toShellDocument(event.info), hasOutline: event.info.hasOutline },
        },
        false,
      );
    case "passwordNeeded":
      return upsert(
        state,
        { tab: event.tab, displayName: event.displayName, content: { kind: "password", wrong: event.wrong } },
        false,
      );
    case "failed":
      return upsert(
        state,
        { tab: event.tab, displayName: event.displayName, content: { kind: "error", code: event.error.code } },
        false,
      );
    case "tabLimit":
      return { ...state, notice: { kind: "tabLimit", ignoredFiles: event.ignoredFiles } };
  }
}
