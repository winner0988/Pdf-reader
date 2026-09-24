// How links are described to the user (MVP-12, docs/ux/screen-map.md, section 4). A PDF
// controls its URIs completely, so nothing it hides may stay hidden.

import { strings } from "@/i18n/zh-TW";
import type { LinkTarget } from "@/ipc/generated/contract";

/** Longest link text shown in the status bar; the dialog always shows everything. */
export const HOVER_MAX_CHARS = 300;

/**
 * Characters that are invisible or change how the text around them is displayed: control
 * characters, bidirectional marks, embeddings, overrides and isolates (U+202E can make
 * "…/fdp.exe" read as "…/exe.pdf"), zero-width characters and the byte order mark. Mirrors
 * `is_invisible_format` in crates/ipc_contract/src/text.rs, plus controls.
 */
export function isHiddenCharacter(char: string): boolean {
  const code = char.codePointAt(0) ?? 0;
  return (
    code <= 0x1f ||
    (code >= 0x7f && code <= 0x9f) ||
    code === 0x061c ||
    (code >= 0x200b && code <= 0x200f) ||
    (code >= 0x202a && code <= 0x202e) ||
    (code >= 0x2060 && code <= 0x2069) ||
    code === 0xfeff
  );
}

/** `text` with every hidden character written out as `[U+XXXX]`. */
export function revealHidden(text: string): string {
  let out = "";
  for (const char of text) {
    out += isHiddenCharacter(char)
      ? `[U+${char.codePointAt(0)!.toString(16).toUpperCase().padStart(4, "0")}]`
      : char;
  }
  return out;
}

export function hasHiddenCharacters(text: string): boolean {
  for (const char of text) if (isHiddenCharacter(char)) return true;
  return false;
}

/** What the status bar says while the pointer is over a link (also its accessible name). */
export function linkHoverText(target: LinkTarget): string {
  switch (target.kind) {
    case "page":
      return strings.links.hoverPage(target.pageIndex + 1);
    case "uri": {
      const shown = revealHidden(target.uri);
      return shown.length > HOVER_MAX_CHARS ? `${shown.slice(0, HOVER_MAX_CHARS)}…` : shown;
    }
    case "blocked":
      return strings.links.hoverBlocked(strings.links.blocked[target.action].label);
  }
}
