// E2E tests for retry_open and cancel (#432, #495).
// - retry_open: reopens a tab that failed to open (e.g. file was missing or unreadable) once it can be accessed.
// - cancel: stops an ongoing cancellable operation (e.g. export), and safely ignores non-existent
//   or already-finished request IDs without disrupting running operations.

import { copyFileSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, documentTabs, expect, launchAgain, pdfWorkerPids, stopProcess, test } from "./app";

async function openExport(page: Page) {
  await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.export}`) }).click();
  return page.getByRole("dialog", { name: strings.export.title });
}

test("when a tab's worker dies, the document recovers in a new worker while another tab's worker is untouched (ADR 0012) (#432)", async ({
  launch,
}) => {
  // 1. Open two documents from corpus; each tab has its own worker.
  const page = await launch(corpus("benign/single-page.pdf"));
  await expect(documentTabs(page)).toHaveCount(1);
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Record worker PID for the first tab
  await expect.poll(() => pdfWorkerPids(page).length).toBe(1);
  const firstWorkerPid = pdfWorkerPids(page)[0]!;

  // Open second document in a new tab
  launchAgain(corpus("benign/outline-3-levels.pdf"));
  await expect(documentTabs(page)).toHaveCount(2);
  await expect(page.getByRole("contentinfo")).toContainText("outline-3-levels.pdf");
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // The newly added worker belongs to the second tab
  await expect.poll(() => pdfWorkerPids(page).length).toBe(2);
  const workersAfterSecond = pdfWorkerPids(page);
  const secondWorkerPid = workersAfterSecond.find((pid) => pid !== firstWorkerPid)!;
  expect(secondWorkerPid).toBeDefined();

  // Switch back to the first tab
  const tabs = documentTabs(page);
  await tabs.first().click();
  await expect(page.getByRole("contentinfo")).toContainText("single-page.pdf");
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // 2. Kill the first tab's worker process
  stopProcess(firstWorkerPid);

  // Send a request from that tab: rotate the page via toolbar button.
  // Because the single-page document is already mounted and idle, text/links/annotations/fields
  // hooks do not re-run on rotation; exactly one render request is dispatched to the dead worker,
  // guaranteeing the failure is observed on this page rather than racing with other requests.
  await page.getByRole("button", { name: strings.toolbar.rotateCw }).click();

  // Confirm the page display fails with retry button
  const page1 = page.getByRole("img", { name: strings.canvas.page(1) });
  await expect(page1.getByRole("alert")).toContainText(strings.canvas.pageRenderFailed);
  const retryButton = page1.getByRole("button", { name: strings.error.retry });
  await expect(retryButton).toBeVisible();

  // 3. Click retry: document recovers in a new worker
  await retryButton.click();
  await expect(page1).toHaveAttribute("data-state", "ready");

  // Confirm worker was replaced: still 2 workers, second is intact, first is replaced
  await expect.poll(() => pdfWorkerPids(page).length).toBe(2);
  const workersAfterRetry = pdfWorkerPids(page);
  expect(workersAfterRetry).toContain(secondWorkerPid);
  expect(workersAfterRetry).not.toContain(firstWorkerPid);

  // 4. Verify second tab remained usable throughout
  await tabs.nth(1).click();
  await expect(page.getByRole("contentinfo")).toContainText("outline-3-levels.pdf");
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  expect(pdfWorkerPids(page)).toContain(secondWorkerPid);
});

test("retrying a tab that failed to open reopens the document in a new worker via retry_open (#495)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-retry-open-"));
  const deferredFile = path.join(folder, "deferred.pdf");
  try {
    // 1. Launch app with a path that does not yet exist.
    // The main process creates the tab in 'Opening' state without checking existence in add(),
    // then load() calls check_file() which fails with Unreadable; the tab shows the error screen.
    const page = await launch(deferredFile);

    await expect(documentTabs(page)).toHaveCount(1);
    await expect(documentTabs(page).first()).toContainText(strings.tabs.failed);

    // Tab-level error alert (in ReaderShell's ErrorState, not a page render error)
    const alert = page.getByRole("alert").filter({ hasText: strings.error.title });
    await expect(alert).toBeVisible();
    await expect(alert).toContainText(strings.error.messages.unreadable);
    const retryButton = alert.getByRole("button", { name: strings.error.retry });
    await expect(retryButton).toBeVisible();

    // No worker process was created since check_file failed before spawning a worker host
    expect(pdfWorkerPids(page)).toHaveLength(0);

    // 2. Put a valid PDF at the deferred path
    copyFileSync(corpus("benign/single-page.pdf"), deferredFile);

    // 3. Click the retry button in the tab-level error alert, and confirm retry_open was sent
    const retryOpenPromise = page.waitForRequest((req) => req.url().startsWith("http://ipc.localhost/retry_open"));
    await retryButton.click();
    await retryOpenPromise;

    // 4. Confirm document opens successfully:
    // Page canvas renders with ready state
    const page1 = page.getByRole("img", { name: strings.canvas.page(1) });
    await expect(page1).toHaveAttribute("data-state", "ready");

    // Status bar shows file name and page 1 of 1
    await expect(page.getByRole("contentinfo")).toContainText("deferred.pdf");
    await expect(page.getByRole("contentinfo")).toContainText(strings.statusBar.pageStatus(1, 1, strings.toolbar.fitWidth));

    // Tab title no longer shows failed mark
    await expect(documentTabs(page).first()).not.toContainText(strings.tabs.failed);
    await expect(alert).toHaveCount(0);

    // A worker was spawned for the opened document
    await expect.poll(() => pdfWorkerPids(page).length).toBe(1);
    expect(pdfWorkerPids(page)[0]!).toBeGreaterThan(0);

    // 5. Verify direct retry_open command validation (rejects invalid tab and non-failed tab)
    await expect(
      page.evaluate(async () => {
        const internals = (window as unknown as {
          __TAURI_INTERNALS__: { invoke(cmd: string, args?: unknown): Promise<unknown> };
        }).__TAURI_INTERNALS__;
        return internals.invoke("retry_open", { tab: 999999 });
      }),
    ).rejects.toThrow();

    await expect(
      page.evaluate(async () => {
        const internals = (window as unknown as {
          __TAURI_INTERNALS__: { invoke(cmd: string, args?: unknown): Promise<unknown> };
        }).__TAURI_INTERNALS__;
        return internals.invoke("retry_open", { tab: 1 });
      }),
    ).rejects.toThrow();
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("cancelling an ongoing export stops the job with no further progress (#432)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-cancel-"));
  try {
    const page = await launch(corpus("benign/multi-page-10.pdf"));
    const dialog = await openExport(page);
    await dialog.getByRole("radio", { name: strings.export.png }).check();
    await dialog.getByRole("combobox", { name: strings.export.resolution }).selectOption("300");
    await dialog.getByRole("textbox", { name: strings.print.pages }).fill("1-10");
    await dialog.getByRole("button", { name: strings.export.start }).click();

    await answerFileDialog(page, { path: folder });

    // Wait for the export to start and show progress
    const progress = dialog.getByRole("status");
    await expect(progress).toBeVisible();

    // Cancel the ongoing export: while exporting, the button is "停止" (strings.export.stop)
    const stopButton = dialog.getByRole("button", { name: strings.export.stop });
    await expect(stopButton).toBeVisible();
    await stopButton.click();

    // Dialog closes and status bar reports export was stopped
    await expect(dialog).toHaveCount(0);
    const statusText = (await page.getByRole("contentinfo").textContent()) ?? "";
    expect(statusText, "export finished before cancel could take effect").not.toContain(strings.export.done(10));
    await expect(page.getByRole("contentinfo")).toContainText(strings.export.stopped(0).slice(0, 4));

    // Pages written is fewer than total (10)
    const writtenCount = readdirSync(folder).length;
    expect(writtenCount, "export completed all 10 pages before cancel took effect").toBeLessThan(10);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("cancelling a non-existent or already-finished request id succeeds and does not affect running jobs (#432, #495)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-cancel-ids-"));
  try {
    const page = await launch(corpus("benign/multi-page-10.pdf"));
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

    // 1. Perform a short export (page 1) to establish an already finished request
    const dialog1 = await openExport(page);
    await dialog1.getByRole("radio", { name: strings.export.png }).check();
    await dialog1.getByRole("combobox", { name: strings.export.resolution }).selectOption("72");
    await dialog1.getByRole("textbox", { name: strings.print.pages }).fill("1");

    const export1Promise = page.waitForRequest((req) => req.url().startsWith("http://ipc.localhost/export_pages"));
    await dialog1.getByRole("button", { name: strings.export.start }).click();
    await answerFileDialog(page, { path: folder });

    const export1Req = await export1Promise;
    let finishedRequestId: number | undefined;
    const postData = export1Req.postData();
    if (postData) {
      try {
        const parsed = JSON.parse(postData) as { args?: { request?: number } };
        finishedRequestId = parsed.args?.request;
      } catch {
        // Not JSON
      }
    }

    // Wait for the first export to complete
    await expect(page.getByRole("contentinfo")).toContainText(strings.export.done(1));
    await expect(dialog1).toHaveCount(0);

    // 2. Start a second export (pages 2-3)
    const dialog2 = await openExport(page);
    await dialog2.getByRole("radio", { name: strings.export.png }).check();
    await dialog2.getByRole("combobox", { name: strings.export.resolution }).selectOption("72");
    await dialog2.getByRole("textbox", { name: strings.print.pages }).fill("2-3");
    await dialog2.getByRole("button", { name: strings.export.start }).click();

    await answerFileDialog(page, { path: folder });

    // Send cancel for non-existent id (0: counter starts at 1, 0 is never issued) and finished id
    await page.evaluate(async (finishedId) => {
      const internals = (window as unknown as {
        __TAURI_INTERNALS__: { invoke(cmd: string, args?: unknown): Promise<unknown> };
      }).__TAURI_INTERNALS__;
      // Non-existent request id (counter starts at 1, 0 is never issued)
      await internals.invoke("cancel", { request: 0 });
      if (typeof finishedId === "number") {
        // Already finished request id
        await internals.invoke("cancel", { request: finishedId });
      }
    }, finishedRequestId);

    // The export running on the document was not affected and completes normally
    await expect(page.getByRole("contentinfo")).toContainText(strings.export.done(2));
    await expect(dialog2).toHaveCount(0);
    expect(readdirSync(folder).sort()).toEqual([
      "multi-page-10-p1.png",
      "multi-page-10-p2.png",
      "multi-page-10-p3.png",
    ]);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
