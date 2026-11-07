// Playwright global teardown for end-to-end tests (QA-02, #484).
// Cleans up any temporary profile or data folders that were still locked when their tests ended.
import { existsSync, rmSync } from "node:fs";
import { cleanupPendingFolders } from "./cleanup";

export default function globalTeardown(): void {
  cleanupPendingFolders();

  const runDir = process.env.PDF_READER_E2E_RUN_DIR;
  if (runDir && existsSync(runDir)) {
    try {
      rmSync(runDir, { recursive: true, force: true });
    } catch {
      // Best-effort cleanup
    }
  }
}
