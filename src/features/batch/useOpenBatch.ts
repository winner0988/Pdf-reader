import { createContext, useContext } from "react";

/** Opens the batch dialog (B3-04); none without the main process. Provided by `BatchHost`. */
export const OpenBatch = createContext<(() => void) | undefined>(undefined);

/** What the menu's "批次處理…" calls; none (so the item is disabled) outside a `BatchHost` with an API. */
export function useOpenBatch(): (() => void) | undefined {
  return useContext(OpenBatch);
}
