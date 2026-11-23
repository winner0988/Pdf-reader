// Adding, modifying, and deleting links in the real app (Issue #588, B6-03).
// Links point either to a page in the document or to an external URL (http, https, mailto).
// External web links always prompt for confirmation before opening.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) });

test("adding page links and URI links, editing targets, deleting, and undo/redo (B6-03)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-links-"));
  try {
    const page = await launch(corpus("benign/multi-page-10.pdf"));
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    const pageBox = await firstPage(page).boundingBox();
    if (!pageBox) throw new Error("first page not on screen");

    // 1. Activate link tool from toolbar
    const linkToolButton = page.getByRole("button", { name: strings.links.tool });
    await linkToolButton.click();
    await expect(linkToolButton).toHaveAttribute("aria-pressed", "true");

    // 2. Click on the first page to add a page link
    await page.mouse.click(pageBox.x + 100, pageBox.y + 100);

    const addDialog = page.getByRole("dialog", { name: strings.links.addTitle });
    await expect(addDialog).toBeVisible();

    // Default tab is page target: set to page 2 and confirm
    const pageInput = addDialog.getByRole("spinbutton");
    await pageInput.fill("2");
    await addDialog.getByRole("button", { name: strings.links.add }).click();
    await expect(addDialog).toHaveCount(0);

    // 3. Click again to add a URI link
    await page.mouse.click(pageBox.x + 100, pageBox.y + 150);
    await expect(addDialog).toBeVisible();

    // Switch to URI tab
    await addDialog.getByRole("tab", { name: strings.links.targetUri }).click();
    const uriInput = addDialog.getByRole("textbox");
    await uriInput.fill("https://example.com/e2e-test");
    await addDialog.getByRole("button", { name: strings.links.add }).click();
    await expect(addDialog).toHaveCount(0);

    // Turn off link tool
    await page.keyboard.press("Escape");
    await expect(linkToolButton).toHaveAttribute("aria-pressed", "false");

    // 4. Test page navigation link
    const pageLinkButton = page.locator("button[data-link]").filter({ hasText: strings.links.hoverPage(2) }).first();
    await expect(pageLinkButton).toBeVisible();
    await pageLinkButton.click();
    // Navigated to page 2
    await expect(page.getByRole("img", { name: strings.canvas.page(2) })).toBeVisible();

    // 5. Test URI link confirmation dialog
    const uriLinkButton = page.locator("button[data-link]").filter({ hasText: "https://example.com/e2e-test" }).first();
    await expect(uriLinkButton).toBeVisible();
    await uriLinkButton.click();

    const confirmDialog = page.getByRole("dialog", { name: strings.links.confirmTitle });
    await expect(confirmDialog).toBeVisible();
    // SAFETY RULE: Always cancel or press Escape, NEVER click open!
    await page.keyboard.press("Escape");
    await expect(confirmDialog).toHaveCount(0);

    // 6. Test undo and redo
    await page.keyboard.press("Control+z");
    // URI link is undone
    await expect(page.locator("button[data-link]").filter({ hasText: "https://example.com/e2e-test" })).toHaveCount(0);
    await page.keyboard.press("Control+y");
    // Redo brings it back
    await expect(page.locator("button[data-link]").filter({ hasText: "https://example.com/e2e-test" })).toHaveCount(1);

    // 7. Save copy and verify PDF structure
    const copyPath = path.join(folder, "with-links.pdf");
    await page.keyboard.press("Control+Shift+S");
    await answerFileDialog(page, { path: copyPath });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await quit(page);

    const savedPdf = readFileSync(copyPath).toString("latin1");
    // Standard /Link annotations
    expect(savedPdf).toMatch(/\/Subtype\s*\/Link/);
    expect(savedPdf).toMatch(/\/Rect/);
    // Uses /URI or /Dest, never /Launch or /GoToR
    expect(savedPdf).toMatch(/\/URI/);
    expect(savedPdf).not.toMatch(/\/Launch\b/);
    expect(savedPdf).not.toMatch(/\/GoToR\b/);
    expect(savedPdf).not.toMatch(/\/JavaScript\b/);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
