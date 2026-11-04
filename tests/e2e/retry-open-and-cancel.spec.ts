// E2E tests for retry_open and cancel (#432).
// - retry_open: reopens a document when its worker crashes or is killed, while other tabs remain usable.
// - cancel: stops an ongoing cancellable operation (e.g. export), and safely ignores non-existent,
//   already-finished, or other-tab request IDs without disrupting running operations.

import { mkdtempSync, readdirSync, rmSync } from "node:fs";
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

test("when a tab's worker dies, the tab shows an error and can be retried while other tabs stay usable (#432)", async ({
  launch,
}) => {
  // 1. Open two documents from corpus; each tab has its own worker.
  const page = await launch(corpus("benign/multi-page-10.pdf"));
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
  await expect(page.getByRole("contentinfo")).toContainText("multi-page-10.pdf");
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // 2. Kill the first tab's worker process
  stopProcess(firstWorkerPid);

  // Send a request from that tab: jump to an unrendered page (page 10) via page number input,
  // which forces a render request to the dead worker rather than relying on overscanned pages.
  const pageInput = page.getByRole("textbox", { name: strings.toolbar.pageNumber });
  await pageInput.fill("10");
  await pageInput.press("Enter");

  // Confirm the page display fails with retry button
  const page10 = page.getByRole("img", { name: strings.canvas.page(10) });
  await expect(page10.getByRole("alert")).toContainText(strings.canvas.pageRenderFailed);
  const retryButton = page10.getByRole("button", { name: strings.error.retry });
  await expect(retryButton).toBeVisible();

  // 3. Click retry: document recovers in a new worker
  await retryButton.click();
  await expect(page10).toHaveAttribute("data-state", "ready");

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

  // 5. Verify direct retry_open command validation (rejects invalid tab and non-failed tab)
  await expect(
    page.evaluate(async () => {
      const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<unknown> } })
        .__TAURI_INTERNALS__;
      return internals.invoke("retry_open", { tab: 999999 });
    }),
  ).rejects.toThrow();

  await expect(
    page.evaluate(async () => {
      const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<unknown> } })
        .__TAURI_INTERNALS__;
      return internals.invoke("retry_open", { tab: 1 });
    }),
  ).rejects.toThrow();
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

test("cancelling non-existent, finished, or another tab's request id succeeds and does not affect running jobs (#432)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-cancel-ids-"));
  try {
    const page = await launch(corpus("benign/multi-page-10.pdf"));
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

    // Open second document to establish another tab
    launchAgain(corpus("benign/single-page.pdf"));
    await expect(documentTabs(page)).toHaveCount(2);

    // Switch back to first tab
    await documentTabs(page).first().click();
    await expect(page.getByRole("contentinfo")).toContainText("multi-page-10.pdf");

    // Start an export on tab 1
    const dialog = await openExport(page);
    await dialog.getByRole("radio", { name: strings.export.png }).check();
    await dialog.getByRole("combobox", { name: strings.export.resolution }).selectOption("72");
    await dialog.getByRole("textbox", { name: strings.print.pages }).fill("1-2");
    await dialog.getByRole("button", { name: strings.export.start }).click();

    await answerFileDialog(page, { path: folder });

    // Send cancel for non-existent id, finished id, and other tab's id via __TAURI_INTERNALS__.invoke
    await page.evaluate(async () => {
      const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<unknown> } })
        .__TAURI_INTERNALS__;
      // 1. Non-existent request id
      await internals.invoke("cancel", { request: 999999 });
      // 2. Already finished request id
      await internals.invoke("cancel", { request: 1 });
      // 3. Another tab's request id
      await internals.invoke("cancel", { request: 2 });
    });

    // The export running on tab 1 was not affected and completes normally
    await expect(page.getByRole("contentinfo")).toContainText(strings.export.done(2));
    await expect(dialog).toHaveCount(0);
    expect(readdirSync(folder).sort()).toEqual(["multi-page-10-p1.png", "multi-page-10-p2.png"]);

    // Tab 2 remains unaffected and functional
    await documentTabs(page).nth(1).click();
    await expect(page.getByRole("contentinfo")).toContainText("single-page.pdf");
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
