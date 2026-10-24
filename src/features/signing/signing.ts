// Signing a document (B3-06): what the user chooses, what is sent to the main process, and what is
// said when it does not work.

import { strings } from "@/i18n/zh-TW";
import {
  LIMITS,
  type DocumentId,
  type ErrorCode,
  type PageCorner,
  type SignArgs,
  type SignCheck,
} from "@/ipc/generated/contract";

const t = strings.signing;

/** The corners offered, in reading order; the last is where a signature usually goes. */
export const CORNERS: readonly PageCorner[] = ["topLeft", "topRight", "bottomLeft", "bottomRight"];

export type SignForm = {
  /** On the page shown, in a corner; `null`: seen on no page. */
  corner: PageCorner | null;
  reason: string;
};

/** What is chosen when the dialog opens: a box in the bottom right corner of the page shown. */
export const DEFAULT_FORM: SignForm = { corner: "bottomRight", reason: "" };

/** Why the document cannot be signed, said when the dialog opens; nothing for one that can be. */
export const REFUSALS: Partial<Record<SignCheck, string>> = {
  encrypted: t.encrypted,
  unsaved: t.unsaved,
  certified: t.certified,
};

/** Whether `reason` is one the main process takes: at most `LIMITS.maxSigningReasonBytes` in UTF-8. */
export function reasonFits(reason: string): boolean {
  return new TextEncoder().encode(reason.trim()).length <= LIMITS.maxSigningReasonBytes;
}

/**
 * What is sent to sign: the page is 1-based here, 0-based there; a reason of spaces is none; the
 * time zone is this computer's, in minutes east of UTC.
 */
export function argsOf(
  doc: DocumentId,
  file: number,
  password: string,
  form: SignForm,
  page: number,
  now: Date = new Date(),
): SignArgs {
  const reason = form.reason.trim();
  return {
    doc,
    file,
    password,
    place: form.corner === null ? null : { pageIndex: page - 1, corner: form.corner },
    reason: reason === "" ? null : reason,
    utcOffsetMinutes: -now.getTimezoneOffset(),
  };
}

/** What the user is told when signing failed: never the password, and nothing was written. */
export function failure(code: ErrorCode): string {
  switch (code) {
    case "certificateUnreadable":
    case "certificateExpired":
    case "certificateNotYetValid":
    case "certificateNotForSigning":
    case "signatureTooLarge":
    case "signatureNotValid":
      return strings.error.messages[code];
    case "encrypted":
      return t.encrypted;
    case "notAllowed":
      return t.certified;
    case "corrupted":
      return t.repaired;
    case "invalidArgument":
      return t.chooseAgain;
    default:
      return t.failed;
  }
}
