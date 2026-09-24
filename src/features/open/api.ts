// Main-process commands for opening documents (docs/architecture/ipc-contract.md, MVP-06).
// The WebView never sees a path: the dialog runs in the main process, and every outcome arrives
// as an OpenEvent carrying a document id and a file name.

import { Channel, invoke } from "@tauri-apps/api/core";

import type { DocumentId, OpenEvent } from "@/ipc/generated/contract";

export type OpenApi = {
  /** Registers `onEvent` for every open event of this page. Safe to call more than once. */
  listen(onEvent: (event: OpenEvent) => void): () => void;
  /** Shows the native open dialog; false if the user cancelled. */
  openDialog(): Promise<boolean>;
  /** Opens the most recently attempted file again. */
  retry(): Promise<void>;
  close(doc: DocumentId): Promise<void>;
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
  retry: () => invoke<void>("retry_open"),
  close: (doc) => invoke<void>("close_document", { doc }),
};
