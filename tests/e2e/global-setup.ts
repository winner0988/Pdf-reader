// Playwright global setup for end-to-end tests (QA-02, #484).
// Creates a unique temporary directory for this test run to store the cleanup manifest,
// passing its path to test workers via environment variables.
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

export default function globalSetup(): void {
  const runDir = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-run-"));
  const manifest = path.join(runDir, "pending-cleanups.txt");
  writeFileSync(manifest, "", "utf8");
  process.env.PDF_READER_E2E_RUN_DIR = runDir;
  process.env.PDF_READER_E2E_CLEANUP_MANIFEST = manifest;
}
