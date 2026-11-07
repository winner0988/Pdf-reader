// Verifies that a locked temporary folder (like WebView2's user data folder on exit)
// does not fail the test during cleanup, records the folder for teardown, and is cleaned
// up by globalTeardown once released (#484).
import { spawn } from "node:child_process";
import { closeSync, existsSync, mkdtempSync, openSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { expect, test } from "@playwright/test";

import { cleanupPendingFolders, getPendingCleanups, removeTemporaryFolder, setCleanupManifest } from "./cleanup";

let initialManifestPath: string;
let initialManifestExists = false;
let initialManifestContent: string | null = null;

test.beforeAll(() => {
  const envManifest = process.env.PDF_READER_E2E_CLEANUP_MANIFEST;
  expect(envManifest, "PDF_READER_E2E_CLEANUP_MANIFEST must be set by globalSetup").toBeTruthy();
  initialManifestPath = envManifest!;
  initialManifestExists = existsSync(initialManifestPath);
  initialManifestContent = initialManifestExists ? readFileSync(initialManifestPath, "utf8") : null;
});

test.afterAll(() => {
  expect(existsSync(initialManifestPath)).toBe(initialManifestExists);
  const currentContent = existsSync(initialManifestPath)
    ? readFileSync(initialManifestPath, "utf8")
    : null;
  expect(currentContent).toBe(initialManifestContent);
});

/**
 * Opens a file with fs.openSync and holds an exclusive read lock via PowerShell so that
 * directory deletion on Windows fails with EBUSY / EPERM (reproducing WebView2's profile lock).
 */
async function lockFile(filePath: string): Promise<() => Promise<void>> {
  const fd = openSync(filePath, "r+");
  let fdClosed = false;
  const safeCloseFd = () => {
    if (!fdClosed) {
      fdClosed = true;
      try {
        closeSync(fd);
      } catch {
        // ignore
      }
    }
  };

  const escapedPath = filePath.replaceAll("'", "''");
  const psScript = `$ErrorActionPreference = 'Stop'; $s = [System.IO.File]::Open('${escapedPath}', [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite); Write-Output READY; [Console]::ReadLine(); $s.Close()`;
  const child = spawn("powershell", ["-NoProfile", "-NonInteractive", "-Command", psScript], {
    stdio: ["pipe", "pipe", "pipe"],
  });

  let stderr = "";
  child.stderr?.on("data", (chunk) => {
    stderr += String(chunk);
  });

  try {
    await new Promise<void>((resolve, reject) => {
      let settled = false;

      child.stdout?.on("data", (data) => {
        if (!settled && String(data).includes("READY")) {
          settled = true;
          resolve();
        }
      });

      child.on("error", (err) => {
        if (!settled) {
          settled = true;
          safeCloseFd();
          reject(new Error(`PowerShell lockFile error: ${err.message}`));
        }
      });

      child.on("exit", (code) => {
        if (!settled) {
          settled = true;
          safeCloseFd();
          reject(
            new Error(
              `PowerShell process exited before READY (code ${code}): ${stderr.trim()}`,
            ),
          );
        }
      });
    });
  } catch (error) {
    safeCloseFd();
    throw error;
  }

  return async () => {
    if (child.stdin?.writable) {
      child.stdin.write("\n");
    }
    safeCloseFd();
    if (child.exitCode === null) {
      await new Promise((resolve) => child.on("exit", resolve));
    }
  };
}

test("a locked folder does not throw on cleanup and is removed by teardown (#484)", async () => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-cleanup-test-'q-"));
  setCleanupManifest(path.join(folder, "pending-cleanups.txt"));
  let unlock: (() => Promise<void>) | null = null;
  try {
    const filePath = path.join(folder, "locked.txt");
    writeFileSync(filePath, "locked content", "utf8");

    unlock = await lockFile(filePath);

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

    await unlock();
    unlock = null;

    // Step 2: global teardown cleans up all recorded pending folders
    cleanupPendingFolders({ maxRetries: 5, retryDelay: 20 });

    // The folder is now deleted
    expect(existsSync(folder)).toBe(false);
  } finally {
    try {
      if (unlock) {
        await unlock();
      }
    } finally {
      setCleanupManifest(null);
      if (existsSync(folder)) {
        rmSync(folder, { recursive: true, force: true });
      }
    }
  }
});

test("removeTemporaryFolder re-throws non-lock errors", () => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-cleanup-test-err-"));
  setCleanupManifest(path.join(folder, "pending-cleanups.txt"));
  try {
    // Pass an invalid option or cause an error that is not EBUSY/EPERM/ENOTEMPTY.
    // Calling rmSync with invalid type or unhandled error throws.
    expect(() => {
      removeTemporaryFolder(folder, { maxRetries: -1 });
    }).toThrow();
  } finally {
    try {
      rmSync(folder, { recursive: true, force: true });
    } finally {
      setCleanupManifest(null);
    }
  }
});
