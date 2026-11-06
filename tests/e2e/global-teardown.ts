// Playwright global teardown for end-to-end tests (QA-02, #484).
// Cleans up any temporary profile or data folders that were still locked when their tests ended.
import { cleanupPendingFolders } from "./cleanup";

export default function globalTeardown(): void {
  cleanupPendingFolders();
}
