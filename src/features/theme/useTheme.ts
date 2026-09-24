import { useEffect, useState } from "react";

export type ThemePreference = "system" | "light" | "dark";

const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * Applies the theme by toggling the `dark` class on <html>. Follows the system by default.
 * Not persisted yet (settings storage is a later card).
 */
export function useTheme(): [ThemePreference, (preference: ThemePreference) => void] {
  const [preference, setPreference] = useState<ThemePreference>("system");

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
