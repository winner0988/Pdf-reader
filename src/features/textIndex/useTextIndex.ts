import { createContext, useContext } from "react";

import type { TextIndexApi } from "@/features/textIndex/api";

/** The text index (B3-21): its commands, and opening the dialog that searches it. */
export type TextIndexAccess = {
  api: TextIndexApi;
  openSearchAll: () => void;
};

/** Provided by `TextIndexHost`; none without the main process. */
export const TextIndexContext = createContext<TextIndexAccess | undefined>(undefined);

/** None (so the menu item is disabled and the settings have no section) outside a `TextIndexHost` with an API. */
export function useTextIndex(): TextIndexAccess | undefined {
  return useContext(TextIndexContext);
}
