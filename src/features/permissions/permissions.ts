// What the document's author allows (MVP-19): an encrypted PDF can forbid copying its text and
// printing it, or allow only low-resolution printing. The app obeys, as Adobe Acrobat does.
// The permissions are the author's wish, not a security boundary (docs/architecture/encryption.md).

import { strings } from "@/i18n/zh-TW";
import type { DocumentPermissions } from "@/ipc/generated/contract";

export type { DocumentPermissions };

/** What an unencrypted document (and demo data) allows. */
export const ALL_PERMISSIONS: DocumentPermissions = { copy: true, print: true, printHighQuality: true, modify: true, assemble: true, annotate: true, fillForms: true };

/** The status bar's note of what is restricted, or null when nothing is. */
export function restrictionSummary(permissions: DocumentPermissions): string | null {
  const t = strings.permissions;
  const restricted: string[] = [];
  if (!permissions.copy) restricted.push(t.noCopy);
  if (!permissions.print) restricted.push(t.noPrint);
  else if (!permissions.printHighQuality) restricted.push(t.lowResPrint);
  return restricted.length > 0 ? t.restricted(restricted.join("、")) : null;
}
