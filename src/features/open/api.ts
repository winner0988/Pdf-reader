// Main-process commands for opening documents in tabs (docs/architecture/ipc-contract.md, MVP-06,
// MVP-14). The WebView never sees a path: the dialog runs in the main process, and every outcome
// arrives as an OpenEvent carrying a tab id, a document id and a file name.

import { Channel, invoke } from "@tauri-apps/api/core";

import type { OpenEvent, TabId, UnlockArgs } from "@/ipc/generated/contract";

export type OpenApi = {
  /** Registers `onEvent` for every open event of this page. Safe to call more than once. */
  listen(onEvent: (event: OpenEvent) => void): () => void;
  /** Shows the native open dialog (several files can be picked); false if the user cancelled. */
  openDialog(): Promise<boolean>;
  /** Opens the file of a tab that failed to open again, in the same tab. */
  retry(tab: TabId): Promise<void>;
  /**
   * Tries a password on a tab whose file is encrypted (MVP-16); the outcome arrives as an open
   * event. The main process hands it to that tab's worker only and keeps nothing.
   */
  unlock(tab: TabId, password: string): Promise<void>;
  /** Closes a tab; its worker ends. */
  close(tab: TabId): Promise<void>;
  /** The tab the window shows, for the window title. */
  setActive(tab: TabId | null): Promise<void>;
};

/**
 * Fans one main-process subscription out to any number of listeners. The main process keeps
 * only the latest channel, so a page must subscribe once (React may run effects twice).
 */
export function createEventHub(subscribe: (onEvent: (event: OpenEvent) => void) => Promise<void>) {
  const listeners = new Set<(event: OpenEvent) => void>();
  let subscribed: Promise<void> | null = null;
  return (listener: (event: OpenEvent) => void) => {
    listeners.add(listener);
    subscribed ??= subscribe((event) => listeners.forEach((notify) => notify(event))).catch((error: unknown) => {
      // Outside Tauri (plain `vite` in a browser) there is no main process to talk to.
      if (import.meta.env.DEV) console.warn("open events unavailable", error);
    });
    return () => {
      listeners.delete(listener);
    };
  };
}

export const tauriOpenApi: OpenApi = {
  listen: createEventHub(async (onEvent) => {
    const channel = new Channel<OpenEvent>();
    channel.onmessage = onEvent;
    await invoke("subscribe_open_events", { onEvent: channel });
  }),
  openDialog: () => invoke<boolean>("open_document_dialog"),
  retry: (tab) => invoke<void>("retry_open", { tab }),
  unlock: (tab, password) => invoke<void>("unlock_tab", { args: { tab, password } satisfies UnlockArgs }),
  close: (tab) => invoke<void>("close_tab", { tab }),
  setActive: (tab) => invoke<void>("set_active_tab", { tab }),
};
