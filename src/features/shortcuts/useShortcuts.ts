import { useEffect, useRef } from "react";

import { findShortcut, type ShortcutId } from "@/features/shortcuts/registry";

/** A handler that returns `false` did not use the key: the browser's default happens. */
export type ShortcutHandlers = Partial<Record<ShortcutId, () => void | boolean>>;

/**
 * Calls the handler registered for a shortcut and suppresses the browser default. While not
 * `enabled` (a tab that is not shown, MVP-14) no key is handled.
 */
export function useShortcuts(handlers: ShortcutHandlers, enabled = true): void {
  // Keep the listener stable while always calling the latest handlers.
  const latest = useRef({ handlers, enabled });
  useEffect(() => {
    latest.current = { handlers, enabled };
  });

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      // A key is handled once. When closing a tab shows the next one, React re-renders between
      // two listeners of the same key press, so without this the next tab would close too.
      // It also leaves keys a focused control already used (arrows in the tab bar) alone.
      if (event.defaultPrevented || !latest.current.enabled) return;
      const shortcut = findShortcut(event);
      const handler = shortcut && latest.current.handlers[shortcut.id];
      if (handler && handler() !== false) event.preventDefault();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
