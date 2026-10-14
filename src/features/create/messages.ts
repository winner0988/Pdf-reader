// What the user is told when no PDF could be made of the files (B3-03).

import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";

/** The text for the failure's code in this dialog, else the general one for that code. */
export function createFailure(failure: unknown): string {
  const code = errorCodeOf(failure);
  return strings.create.failures[code] ?? (strings.error.messages[code] || strings.create.failed);
}
