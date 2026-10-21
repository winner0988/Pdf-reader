// Tabs (MVP-14): launching the app again, as Explorer does for a PDF, adds the file to the running
// window as a tab, and every tab keeps its own document and view.
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, launchAgain, test } from "./app";

test("a second launch opens its file in a new tab of the running window", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  // Only the shown tab is in the accessibility tree: hidden tabs are display: none.
  const status = page.getByRole("contentinfo");
  await expect(status).toContainText("multi-page-10.pdf");

  launchAgain(corpus("benign/outline-3-levels.pdf"));

  const tabs = page.getByRole("tablist", { name: strings.tabs.label }).getByRole("tab");
  await expect(tabs).toHaveCount(2);
  await expect(status).toContainText("outline-3-levels.pdf");
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // The first tab still shows its own document.
  await tabs.first().click();
  await expect(status).toContainText(strings.statusBar.pageStatus(1, 10, strings.toolbar.fitWidth));
  await expect(status).toContainText("multi-page-10.pdf");

  // Ctrl+W closes the shown tab only.
  await page.keyboard.press("Control+w");
  await expect(tabs).toHaveCount(1);
  await expect(status).toContainText("outline-3-levels.pdf");
});

test("close buttons overlay aligns inside each tab presentation box across states (#188)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-tabs-"));
  try {
    const longName = "very-long-filename-that-exceeds-the-maximum-tab-width-and-gets-truncated.pdf";
    const longFile = path.join(folder, longName);
    copyFileSync(corpus("benign/single-page.pdf"), longFile);

    const editName = "editable-tab-sample.pdf";
    const editFile = path.join(folder, editName);
    copyFileSync(corpus("benign/multi-page-10.pdf"), editFile);

    const lockedName = "encrypted-aes256.pdf";
    const lockedFile = corpus("benign/encrypted-aes256.pdf");

    // Launch with long filename (tab will be truncated)
    const page = await launch(longFile);
    const tablist = page.getByRole("tablist", { name: strings.tabs.label });
    await expect(tablist.getByRole("tab")).toHaveCount(1);
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

    // Open second file to edit (unsaved • mark)
    launchAgain(editFile);
    await expect(tablist.getByRole("tab")).toHaveCount(2);
    const firstPage = page.getByRole("img", { name: strings.canvas.page(1) }).first();
    await expect(firstPage).toHaveAttribute("data-state", "ready");
    const doc = Number(await firstPage.getAttribute("data-doc"));
    await page.evaluate(async (doc) => {
      await (
        window as unknown as {
          __TAURI_INTERNALS__: { invoke: (cmd: string, args: unknown) => Promise<unknown> };
        }
      ).__TAURI_INTERNALS__.invoke("apply_edit", {
        args: { doc, edit: { kind: "rotatePages", pages: [0], by: "cw90" } },
      });
    }, doc);
    await expect(
      page.getByRole("tab", { name: new RegExp(`^editable-tab-sample\\.pdf\\s*${strings.tabs.unsaved}$`) }),
    ).toBeVisible();

    // Open third file (password locked, lock icon)
    launchAgain(lockedFile);
    await expect(tablist.getByRole("tab")).toHaveCount(3);
    await expect(page.getByLabel(strings.password.label, { exact: true })).toBeVisible();

    // Verify close buttons for all tabs are fully enclosed in their corresponding tab slot
    const slots = tablist.locator('[role="presentation"]');
    await expect(slots).toHaveCount(3);
    const fileNames = [longName, editName, lockedName];

    const verifyAlignment = async () => {
      for (let i = 0; i < fileNames.length; i++) {
        const name = fileNames[i];
        const slot = slots.nth(i);
        const tabButton = slot.getByRole("tab");
        await expect(tabButton).toHaveAttribute("title", name);

        const closeButton = page.getByRole("button", { name: strings.tabs.close(name) });
        await expect(closeButton).toBeVisible();

        const slotBox = await slot.boundingBox();
        const closeBox = await closeButton.boundingBox();
        expect(slotBox).not.toBeNull();
        expect(closeBox).not.toBeNull();

        if (slotBox && closeBox) {
          expect(closeBox.x).toBeGreaterThanOrEqual(slotBox.x - 0.5);
          expect(closeBox.y).toBeGreaterThanOrEqual(slotBox.y - 0.5);
          expect(closeBox.x + closeBox.width).toBeLessThanOrEqual(slotBox.x + slotBox.width + 0.5);
          expect(closeBox.y + closeBox.height).toBeLessThanOrEqual(slotBox.y + slotBox.height + 0.5);
        }
      }
    };

    // Check with the password tab currently active
    await verifyAlignment();

    // Switch active tab to the long name tab and verify alignment remains exact
    await slots.first().getByRole("tab").click();
    await verifyAlignment();
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
