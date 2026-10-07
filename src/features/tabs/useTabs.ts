import { useCallback, useEffect, useReducer, useRef } from "react";

import { isOcrEvent, type OcrEvent } from "@/features/ocr/model";
import type { OpenApi } from "@/features/open/api";
import { initialTabs, reduceTabs } from "@/features/tabs/model";
import type { DocumentInfo, ErrorCode, IpcError, TabId } from "@/ipc/generated/contract";

function errorCode(error: unknown): ErrorCode {
  const code = (error as Partial<IpcError> | null)?.code;
  return typeof code === "string" ? code : "internal";
}

/**
 * Connects the window's tabs to the main process (MVP-14): open events in; open, retry, close
 * and "which tab is shown" commands out. `onOcr` hears how recognising the text of scanned pages
 * goes (B2-10), which comes on the same channel but is not about the tabs themselves.
 */
export function useTabs(api: OpenApi, onOcr?: (event: OcrEvent) => void) {
  const [state, dispatch] = useReducer(reduceTabs, initialTabs);
  /**
   * Each open tab's document as the main process last described it. The page renders an event a
   * moment after it arrives; a command issued in between (the next field of a form, saving) must
   * not use the document the page was still showing.
   */
  const latest = useRef(new Map<TabId, DocumentInfo>());

  const ocr = useRef(onOcr);
  useEffect(() => {
    ocr.current = onOcr;
  });

  useEffect(
    () =>
      api.listen((event) => {
        if (isOcrEvent(event)) {
          // Not a change of the tab: what the tab last said about its document stands.
          ocr.current?.(event);
          return;
        }
        if (event.kind === "opened") latest.current.set(event.tab, event.info);
        else if ("tab" in event) latest.current.delete(event.tab);
        dispatch({ type: "event", event });
      }),
    [api],
  );

  // The main process titles the window after the tab it shows.
  useEffect(() => {
    api.setActive(state.active).catch(() => {});
  }, [api, state.active]);

  const open = useCallback(() => {
    // The outcome arrives as open events; only a failing dialog is reported here.
    api.openDialog().catch((error: unknown) => dispatch({ type: "failed", code: errorCode(error) }));
  }, [api]);

  const retry = useCallback(
    (tab: TabId) => {
      api.retry(tab).catch((error: unknown) => dispatch({ type: "failed", code: errorCode(error) }));
    },
    [api],
  );

  const unlock = useCallback(
    (tab: TabId, password: string) => {
      api.unlock(tab, password).catch((error: unknown) => dispatch({ type: "failed", code: errorCode(error) }));
    },
    [api],
  );

  const close = useCallback(
    (tab: TabId) => {
      // The tab goes away either way; a document the worker already lost needs no release.
      dispatch({ type: "closed", tab });
      latest.current.delete(tab);
      api.close(tab).catch(() => {});
    },
    [api],
  );

  const latestInfo = useCallback((tab: TabId) => latest.current.get(tab), []);
  const activate = useCallback((tab: TabId) => dispatch({ type: "activate", tab }), []);
  const step = useCallback((by: 1 | -1) => dispatch({ type: "step", by }), []);
  const dismissNotice = useCallback(() => dispatch({ type: "dismissNotice" }), []);
  const dismissCloseRequest = useCallback(() => dispatch({ type: "dismissCloseRequest" }), []);

  return { state, open, retry, unlock, close, latestInfo, activate, step, dismissNotice, dismissCloseRequest };
}
