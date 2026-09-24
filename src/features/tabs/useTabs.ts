import { useCallback, useEffect, useReducer } from "react";

import type { OpenApi } from "@/features/open/api";
import { initialTabs, reduceTabs } from "@/features/tabs/model";
import type { ErrorCode, IpcError, TabId } from "@/ipc/generated/contract";

function errorCode(error: unknown): ErrorCode {
  const code = (error as Partial<IpcError> | null)?.code;
  return typeof code === "string" ? code : "internal";
}

/**
 * Connects the window's tabs to the main process (MVP-14): open events in; open, retry, close
 * and "which tab is shown" commands out.
 */
export function useTabs(api: OpenApi) {
  const [state, dispatch] = useReducer(reduceTabs, initialTabs);

  useEffect(() => api.listen((event) => dispatch({ type: "event", event })), [api]);

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

  const close = useCallback(
    (tab: TabId) => {
      // The tab goes away either way; a document the worker already lost needs no release.
      dispatch({ type: "closed", tab });
      api.close(tab).catch(() => {});
    },
    [api],
  );

  const activate = useCallback((tab: TabId) => dispatch({ type: "activate", tab }), []);
  const step = useCallback((by: 1 | -1) => dispatch({ type: "step", by }), []);
  const dismissNotice = useCallback(() => dispatch({ type: "dismissNotice" }), []);

  return { state, open, retry, close, activate, step, dismissNotice };
}
