import { useEffect, useRef } from "react";

import { findShortcut, type ShortcutId } from "@/features/shortcuts/registry";

export type ShortcutHandlers = Partial<Record<ShortcutId, () => void>>;

/** Calls the handler registered for a shortcut and suppresses the browser default. */
export function useShortcuts(handlers: ShortcutHandlers): void {
  // Keep the listener stable while always calling the latest handlers.
  const latest = useRef(handlers);
  useEffect(() => {
    latest.current = handlers;
  });

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const shortcut = findShortcut(event);
      const handler = shortcut && latest.current[shortcut.id];
      if (handler) {
        event.preventDefault();
        handler();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
