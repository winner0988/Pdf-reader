import { createContext, useContext } from "react";

import type { Settings } from "@/features/settings/api";

export type SettingsContextValue = {
  settings: Settings;
  /** Changes some settings; the whole set is saved. */
  update: (change: Partial<Settings>) => void;
  /** The last change could not be saved: it applies, but not after a restart. */
  saveFailed: boolean;
};

export const SettingsContext = createContext<SettingsContextValue | null>(null);

/** The window's settings (B2-12), or null outside a `SettingsProvider` (tests, demo data). */
export function useSettings(): SettingsContextValue | null {
  return useContext(SettingsContext);
}
