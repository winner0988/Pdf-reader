import { useCallback, useEffect, useReducer } from "react";

import type { OpenApi } from "@/features/open/api";
import { initialSession, reduceSession } from "@/features/open/session";
import type { ErrorCode, IpcError } from "@/ipc/generated/contract";

function errorCode(error: unknown): ErrorCode {
  const code = (error as Partial<IpcError> | null)?.code;
  return typeof code === "string" ? code : "internal";
}

/** Connects the window to the main process: open events in, open/close/retry commands out. */
export function useOpenSession(api: OpenApi) {
  const [session, dispatch] = useReducer(reduceSession, initialSession);

  useEffect(() => api.listen((event) => dispatch({ type: "event", event })), [api]);

  const open = useCallback(() => {
    // The outcome arrives as open events; only a failing dialog is reported here.
    api.openDialog().catch((error: unknown) => dispatch({ type: "failed", code: errorCode(error) }));
  }, [api]);

  const retry = useCallback(() => {
    api.retry().catch((error: unknown) => dispatch({ type: "failed", code: errorCode(error) }));
  }, [api]);

  const { doc } = session;
  const close = useCallback(() => {
    if (doc === null) return;
    // The window is emptied either way; a document the worker already lost needs no release.
    api.close(doc).catch(() => {});
    dispatch({ type: "closed" });
  }, [api, doc]);

  const dismissNotice = useCallback(() => dispatch({ type: "dismissNotice" }), []);

  return { session, open, retry, close, dismissNotice };
}
