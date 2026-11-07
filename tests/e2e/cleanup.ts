// Shared temporary folder cleanup and deferred teardown for end-to-end tests (#484).
// WebView2 and child processes occasionally hold locks on user data profiles for a few
// seconds after process exit. Instead of failing the test with EBUSY, removeTemporaryFolder
// logs a warning, records the folder in the run's manifest, and defers final removal to
// Playwright's globalTeardown.
import { appendFileSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import path from "node:path";

export type RemoveFolderOptions = {
  /** Maximum retry attempts before giving up (default: 10, ~11s total because Node's rmSync increases delay by retryDelay on each attempt). */
  maxRetries?: number;
  /** Delay between retries in milliseconds (default: 200ms). */
  retryDelay?: number;
};

let customManifestPath: string | null = null;

/** Override the cleanup manifest file location (primarily for testing). */
export function setCleanupManifest(manifestPath: string | null): void {
  customManifestPath = manifestPath;
}

/** Returns the active cleanup manifest file path, if configured. */
export function getManifestPath(): string | null {
  return customManifestPath ?? process.env.PDF_READER_E2E_CLEANUP_MANIFEST ?? null;
}

/**
 * Records a folder for deferred cleanup by globalTeardown.
 *
 * Writes to the run's cleanup manifest file so that globalTeardown
 * (which runs in a separate process from test workers) knows which
 * folders belong to this test run.
 */
export function recordPendingCleanup(folder: string): void {
  const resolved = path.resolve(folder);

  const manifest = getManifestPath();
  if (manifest) {
    try {
      mkdirSync(path.dirname(manifest), { recursive: true });
      appendFileSync(manifest, `${resolved}\n`, "utf8");
    } catch (error) {
      console.warn(`warning: could not record pending cleanup to "${manifest}": ${String(error)}`);
    }
  } else {
    console.warn(`warning: could not record pending cleanup for "${resolved}" because no cleanup manifest is configured`);
  }
}

/** Returns all folders currently recorded for deferred cleanup in the manifest file. */
export function getPendingCleanups(): string[] {
  const folders = new Set<string>();
  const manifest = getManifestPath();
  if (manifest && existsSync(manifest)) {
    try {
      const content = readFileSync(manifest, "utf8");
      for (const line of content.split(/\r?\n/)) {
        const trimmed = line.trim();
        if (trimmed) {
          folders.add(trimmed);
        }
      }
    } catch (error) {
      console.warn(`warning: could not read cleanup manifest "${manifest}": ${String(error)}`);
    }
  }
  return Array.from(folders);
}

/**
 * Removes a temporary folder. Retries for ~11 seconds by default
 * (maxRetries: 10, retryDelay: 200; Node's rmSync adds retryDelay per attempt: 1+2+...+10 = 55 * 200ms = 11s).
 *
 * If the folder is still locked (EBUSY, EPERM, ENOTEMPTY), logs a warning and records
 * the folder for globalTeardown rather than throwing and failing the test. Any other
 * filesystem error is re-thrown.
 */
export function removeTemporaryFolder(folder: string, options?: RemoveFolderOptions): void {
  if (!folder || !existsSync(folder)) {
    return;
  }

  const maxRetries = options?.maxRetries ?? 10;
  const retryDelay = options?.retryDelay ?? 200;

  try {
    rmSync(folder, { recursive: true, force: true, maxRetries, retryDelay });
  } catch (error: unknown) {
    const code = (error as NodeJS.ErrnoException)?.code;
    if (code === "EBUSY" || code === "EPERM" || code === "ENOTEMPTY") {
      console.warn(`warning: could not remove temporary folder "${folder}" (${code}), deferring to teardown`);
      recordPendingCleanup(folder);
    } else {
      throw error;
    }
  }
}

/**
 * Cleans up all pending temporary folders recorded during the test run.
 * Called during Playwright's globalTeardown.
 */
export function cleanupPendingFolders(options?: RemoveFolderOptions): void {
  const folders = getPendingCleanups();
  const maxRetries = options?.maxRetries ?? 10;
  const retryDelay = options?.retryDelay ?? 200;

  for (const folder of folders) {
    if (!existsSync(folder)) {
      continue;
    }
    try {
      rmSync(folder, { recursive: true, force: true, maxRetries, retryDelay });
    } catch (error) {
      const code = (error as NodeJS.ErrnoException)?.code ?? String(error);
      console.warn(`warning: teardown could not remove temporary folder "${folder}" (${code})`);
    }
  }

  const manifest = getManifestPath();
  if (manifest && existsSync(manifest)) {
    try {
      rmSync(manifest, { force: true });
    } catch {
      // Best-effort cleanup
    }
  }
}
