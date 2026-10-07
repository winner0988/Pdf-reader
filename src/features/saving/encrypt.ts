// What is asked for when a copy of the document is encrypted (B2-15, docs/architecture/encrypt-copy.md):
// the same rules the main process checks, so the dialog can say what is wrong before anything is sent.

import { LIMITS, type DocumentId, type EncryptArgs, type Restrictions } from "@/ipc/generated/contract";

export type EncryptForm = {
  open: string;
  openAgain: string;
  permissions: string;
  permissionsAgain: string;
  restrictions: Restrictions;
};

export const EMPTY_FORM: EncryptForm = {
  open: "",
  openAgain: "",
  permissions: "",
  permissionsAgain: "",
  restrictions: { print: false, copy: false, modify: false },
};

export type Problem =
  | "nothing"
  | "openMismatch"
  | "permissionsMismatch"
  | "permissionsNeeded"
  | "same"
  | "tooLong";

const bytes = (text: string) => new TextEncoder().encode(text).length;

const anyRestriction = (restrictions: Restrictions) => restrictions.print || restrictions.copy || restrictions.modify;

/** The first thing wrong with `form`, or `null` if it can be sent. */
export function problemWith(form: EncryptForm): Problem | null {
  if (bytes(form.open) > LIMITS.maxNewPasswordBytes || bytes(form.permissions) > LIMITS.maxNewPasswordBytes) {
    return "tooLong";
  }
  if (form.open !== form.openAgain) return "openMismatch";
  if (form.permissions !== form.permissionsAgain) return "permissionsMismatch";
  if (anyRestriction(form.restrictions) && form.permissions === "") return "permissionsNeeded";
  if (form.open !== "" && form.open === form.permissions) return "same";
  if (form.open === "" && !anyRestriction(form.restrictions)) return "nothing";
  return null;
}

/** The arguments of `encrypt_copy` for a form with nothing wrong with it. */
export function argsOf(doc: DocumentId, form: EncryptForm): EncryptArgs {
  return {
    doc,
    openPassword: form.open === "" ? null : form.open,
    permissionsPassword: form.permissions === "" ? null : form.permissions,
    restrictions: form.restrictions,
  };
}
