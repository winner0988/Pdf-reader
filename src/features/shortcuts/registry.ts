// Central keyboard shortcut registry (docs/ux/screen-map.md, section 7).
// Components never listen for keys themselves; they pass handlers to useShortcuts().

import { strings } from "@/i18n/zh-TW";

export type ShortcutId = keyof typeof strings.shortcuts.descriptions;

export type Shortcut = {
  id: ShortcutId;
  /** Key labels shown in the help dialog. */
  keys: string[];
  matches: (event: KeyboardEvent) => boolean;
};

const ctrl = (event: KeyboardEvent) => event.ctrlKey && !event.altKey;
const plain = (event: KeyboardEvent) => !event.ctrlKey && !event.altKey && !event.metaKey;

export const SHORTCUTS: Shortcut[] = [
  { id: "open", keys: ["Ctrl+O"], matches: (e) => ctrl(e) && e.key.toLowerCase() === "o" },
  { id: "close", keys: ["Ctrl+W"], matches: (e) => ctrl(e) && e.key.toLowerCase() === "w" },
  { id: "search", keys: ["Ctrl+F"], matches: (e) => ctrl(e) && e.key.toLowerCase() === "f" },
  { id: "findNext", keys: ["F3"], matches: (e) => plain(e) && !e.shiftKey && e.key === "F3" },
  { id: "findPrevious", keys: ["Shift+F3"], matches: (e) => plain(e) && e.shiftKey && e.key === "F3" },
  { id: "zoomIn", keys: ["Ctrl+=", "Ctrl++"], matches: (e) => ctrl(e) && (e.key === "=" || e.key === "+") },
  { id: "zoomOut", keys: ["Ctrl+-"], matches: (e) => ctrl(e) && (e.key === "-" || e.key === "_") },
  { id: "fitPage", keys: ["Ctrl+0"], matches: (e) => ctrl(e) && e.key === "0" },
  { id: "actualSize", keys: ["Ctrl+1"], matches: (e) => ctrl(e) && e.key === "1" },
  { id: "fitWidth", keys: ["Ctrl+2"], matches: (e) => ctrl(e) && e.key === "2" },
  { id: "rotateCw", keys: ["Ctrl+]"], matches: (e) => ctrl(e) && e.key === "]" },
  { id: "rotateCcw", keys: ["Ctrl+["], matches: (e) => ctrl(e) && e.key === "[" },
  { id: "goToPage", keys: ["Ctrl+G"], matches: (e) => ctrl(e) && e.key.toLowerCase() === "g" },
  { id: "firstPage", keys: ["Home"], matches: (e) => plain(e) && e.key === "Home" },
  { id: "lastPage", keys: ["End"], matches: (e) => plain(e) && e.key === "End" },
  { id: "toggleSidebar", keys: ["F4"], matches: (e) => plain(e) && e.key === "F4" },
  { id: "nextRegion", keys: ["F6"], matches: (e) => plain(e) && !e.shiftKey && e.key === "F6" },
  { id: "previousRegion", keys: ["Shift+F6"], matches: (e) => plain(e) && e.shiftKey && e.key === "F6" },
  { id: "help", keys: ["Ctrl+/"], matches: (e) => ctrl(e) && e.key === "/" },
];

function isTextInput(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target instanceof HTMLTextAreaElement ||
    (target instanceof HTMLInputElement && !["checkbox", "radio", "button"].includes(target.type))
  );
}

/**
 * Returns the shortcut for a key event, or undefined. While typing in a text field only
 * Ctrl combinations apply, so plain keys such as Home/End keep their editing meaning.
 */
export function findShortcut(event: KeyboardEvent): Shortcut | undefined {
  const shortcut = SHORTCUTS.find((candidate) => candidate.matches(event));
  if (shortcut && isTextInput(event.target) && !event.ctrlKey) {
    return undefined;
  }
  return shortcut;
}
