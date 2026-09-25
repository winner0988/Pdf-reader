// WebView2's own context menu (reload, save as, print the page, …) has no place in the app
// (#80): reloading loses the view, "save as" writes file names and outline titles to disk, and
// printing it prints the app instead of the document. Text fields keep it, for cut, copy and
// paste. The document canvas has the app's own menu (MVP-15), which handles its events first.

import { isTextInput } from "@/features/shortcuts/registry";

/** Suppresses the WebView's context menu everywhere but in text fields; returns the undo. */
export function suppressDefaultContextMenu(target: Window = window): () => void {
  const listener = (event: MouseEvent) => {
    if (!isTextInput(event.target)) event.preventDefault();
  };
  target.addEventListener("contextmenu", listener);
  return () => target.removeEventListener("contextmenu", listener);
}
