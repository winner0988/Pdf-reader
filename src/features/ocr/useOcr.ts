import { useCallback, useReducer } from "react";

import { noOcr, reduceOcr, type OcrEvent } from "@/features/ocr/model";
import type { TabId } from "@/ipc/generated/contract";

/** What every tab's recognising of scanned pages has come to, from the open events (B2-10). */
export function useOcr() {
  const [state, dispatch] = useReducer(reduceOcr, noOcr);
  const handle = useCallback((event: OcrEvent) => dispatch({ type: "event", event }), []);
  const forget = useCallback((tab: TabId) => dispatch({ type: "closed", tab }), []);
  return { state, handle, forget };
}
