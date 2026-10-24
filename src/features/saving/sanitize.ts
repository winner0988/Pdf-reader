// What is asked for when a sanitized copy is made (B3-15, docs/architecture/sanitize.md): the
// options of one document's copy and of a batch of them (B3-19), and what is sent for them.

import type { BatchOperation, SanitizeOptions } from "@/ipc/generated/contract";

export type SanitizeForm = {
  actions: boolean;
  attachments: boolean;
  formScripts: boolean;
  xfa: boolean;
  richMedia: boolean;
  hiddenData: boolean;
  keepWebLinks: boolean;
  annotations: "keep" | "remove" | "flatten";
};

export const DEFAULT_SANITIZE_FORM: SanitizeForm = {
  actions: true,
  attachments: true,
  formScripts: true,
  xfa: true,
  richMedia: true,
  hiddenData: true,
  keepWebLinks: true,
  annotations: "keep",
};

/** The options as the main process takes them, and nothing else. */
export function sanitizeOptionsOf(form: SanitizeForm): SanitizeOptions {
  return {
    actions: form.actions,
    attachments: form.attachments,
    formScripts: form.formScripts,
    xfa: form.xfa,
    richMedia: form.richMedia,
    hiddenData: form.hiddenData,
    keepWebLinks: form.keepWebLinks,
    annotations: form.annotations,
  };
}

/** A batch that makes a sanitized copy of each file with these options (B3-19). */
export function batchSanitizeOf(form: SanitizeForm): BatchOperation {
  return { kind: "sanitize", options: sanitizeOptionsOf(form) };
}
