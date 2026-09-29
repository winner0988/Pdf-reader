import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { DEFAULT_SETTINGS, type Settings, type SettingsApi } from "@/features/settings/api";
import { SettingsContext, type SettingsContextValue } from "@/features/settings/useSettings";

/**
 * The settings of the whole window (B2-12): loaded once from the main process, every change
 * saved there. Every tab reads the same set.
 */
export function SettingsProvider({ api, children }: { api: SettingsApi; children: ReactNode }) {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [saveFailed, setSaveFailed] = useState(false);
  /** A change made before the saved settings arrived wins over them. */
  const changed = useRef(false);

  useEffect(() => {
    let current = true;
    api.get().then(
      (loaded) => {
        if (current && !changed.current) setSettings(loaded);
      },
      // Outside Tauri (plain `vite`) there are no saved settings: the defaults apply.
      () => {},
    );
    return () => {
      current = false;
    };
  }, [api]);

  const value = useMemo<SettingsContextValue>(
    () => ({
      settings,
      saveFailed,
      update: (change) => {
        changed.current = true;
        const next = { ...settings, ...change };
        setSettings(next);
        api.set(next).then(
          () => setSaveFailed(false),
          () => setSaveFailed(true),
        );
      },
    }),
    [api, settings, saveFailed],
  );

  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}
