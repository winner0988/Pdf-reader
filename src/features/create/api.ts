// Making a PDF of files (B3-03, docs/architecture/create-pdf.md). The page only asks: the main
// process shows the system's dialogs for the files and for where the PDF goes, has a sandboxed
// worker read the files, and opens the new PDF in a tab. No path or file content passes through
// the page.

import { invoke } from "@tauri-apps/api/core";

import type { CreateResult } from "@/ipc/generated/contract";

export type CreateApi = {
  /**
   * Asks which files, then where the PDF goes, and makes it. `null` if the user closed either
   * dialog; rejects if the PDF could not be made.
   */
  createPdf(): Promise<CreateResult | null>;
};

export const tauriCreateApi: CreateApi = {
  createPdf: () => invoke<CreateResult | null>("create_pdf"),
};
