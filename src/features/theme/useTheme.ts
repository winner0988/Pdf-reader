import { useEffect, useState } from "react";

import { useSettings } from "@/features/settings/useSettings";
import type { ThemePreference } from "@/ipc/generated/contract";

export type { ThemePreference };

const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * Applies the theme by toggling the `dark` class on <html>. Follows the system by default. Inside
 * a `SettingsProvider` the choice is the window's saved setting (B2-12); without one (tests, demo
 * data) it lasts as long as the component.
 */
export function useTheme(): [ThemePreference, (preference: ThemePreference) => void] {
  const shared = useSettings();
  const [local, setLocal] = useState<ThemePreference>("system");
  const preference = shared ? shared.settings.theme : local;
  const setPreference = shared ? (theme: ThemePreference) => shared.update({ theme }) : setLocal;

  useEffect(() => {
    const media = window.matchMedia(DARK_QUERY);
    const apply = () => {
      const dark = preference === "dark" || (preference === "system" && media.matches);
      document.documentElement.classList.toggle("dark", dark);
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [preference]);

  return [preference, setPreference];
}
