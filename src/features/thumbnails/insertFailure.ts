import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n";

/** What the user is told when the pages of another file cannot be taken in (B2-06, #584). */
export function insertFailureMessage(error: unknown): string {
  const code = errorCodeOf(error);
  if (code === "notAllowed") return strings.pages.sourceNotAllowed;
  if (code === "limitExceeded" || code === "tooLarge") return strings.pages.sourceTooLarge;
  return strings.pages.sourceFailed;
}
