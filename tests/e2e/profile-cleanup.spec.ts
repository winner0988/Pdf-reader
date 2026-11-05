// Verifies that a locked temporary folder (like WebView2's user data folder on exit)
// does not fail the test during cleanup, records the folder for teardown, and is cleaned
// up by globalTeardown once released (#484).
import { spawn } from "node:child_process";
import { closeSync, existsSync, mkdtempSync, openSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { expect, test } from "@playwright/test";

import { cleanupPendingFolders, getPendingCleanups, removeTemporaryFolder } from "./cleanup";

/**
 * Opens a file with fs.openSync and holds an exclusive read lock via PowerShell so that
 * directory deletion on Windows fails with EBUSY / EPERM (reproducing WebView2's profile lock).
 */
async function lockFile(filePath: string): Promise<() => Promise<void>> {
  const fd = openSync(filePath, "r+");
  const psScript = `$s = [System.IO.File]::Open('${filePath}', [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite); Write-Output READY; [Console]::ReadLine(); $s.Close()`;
  const child = spawn("powershell", ["-NoProfile", "-NonInteractive", "-Command", psScript], {
    stdio: ["pipe", "pipe", "ignore"],
  });

  await new Promise<void>((resolve) => {
    child.stdout.on("data", (data) => {
      if (String(data).includes("READY")) resolve();
    });
  });

  return async () => {
    child.stdin.write("\n");
    closeSync(fd);
    await new Promise((resolve) => child.on("exit", resolve));
  };
}

test("a locked folder does not throw on cleanup and is removed by teardown (#484)", async () => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-cleanup-test-"));
  const filePath = path.join(folder, "locked.txt");
  writeFileSync(filePath, "locked content", "utf8");

  const unlock = await lockFile(filePath);
  try {
    const warnings: string[] = [];
    const originalWarn = console.warn;
    console.warn = (...args: unknown[]) => {
      warnings.push(args.map(String).join(" "));
      originalWarn(...args);
    };

    try {
      // Step 1: removeTemporaryFolder should retry, catch EBUSY/EPERM/ENOTEMPTY, warn,
      // record the folder, and NOT throw.
      expect(() => {
        removeTemporaryFolder(folder, { maxRetries: 3, retryDelay: 20 });
      }).not.toThrow();

      // Warning was emitted containing folder path and error code
      expect(warnings.length).toBeGreaterThan(0);
      expect(
        warnings.some(
          (w) =>
            w.includes(folder) &&
            (w.includes("EBUSY") || w.includes("EPERM") || w.includes("ENOTEMPTY")),
        ),
      ).toBe(true);

      // Folder still exists because it was locked
      expect(existsSync(folder)).toBe(true);

      // Folder was recorded for deferred teardown
      const pending = getPendingCleanups();
      expect(pending).toContain(path.resolve(folder));
    } finally {
      console.warn = originalWarn;
    }
  } finally {
    await unlock();
  }

  // Step 2: global teardown cleans up all recorded pending folders
  cleanupPendingFolders({ maxRetries: 5, retryDelay: 20 });

  // The folder is now deleted
  expect(existsSync(folder)).toBe(false);
});

test("removeTemporaryFolder re-throws non-lock errors", () => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-cleanup-test-err-"));
  // Pass an invalid option or cause an error that is not EBUSY/EPERM/ENOTEMPTY.
  // Calling rmSync with invalid type or unhandled error throws.
  expect(() => {
    removeTemporaryFolder(folder, { maxRetries: -1 });
  }).toThrow();
  cleanupPendingFolders();
});
