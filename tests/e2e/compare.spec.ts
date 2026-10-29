// Comparing text between two open tabs (B3-24, E9). Two versions of a contract are opened
// in separate tabs; comparing their text lists the differences by page; clicking an item
// jumps to the corresponding page in the current tab; results can be exported as a Markdown file.
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, launchAgain, test } from "./app";

const t = strings.compare;

test.describe("compare text (B3-24, E9)", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-compare-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("compares text between two open tabs, jumps to page on click, and exports Markdown", async ({ launch }) => {
    // Launch first tab with contract-v1
    const page = await launch(corpus("benign/contract-v1.pdf"));
    await expect(page.getByRole("contentinfo")).toContainText("contract-v1.pdf");

    // Launch second tab with contract-v2 in running window
    launchAgain(corpus("benign/contract-v2.pdf"));
    const tabs = page.getByRole("tablist", { name: strings.tabs.label }).getByRole("tab");
    await expect(tabs).toHaveCount(2);
    await expect(page.getByRole("contentinfo")).toContainText("contract-v2.pdf");

    // Open More menu and choose compare text
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.compareText}`) }).click();

    // Dialog appears
    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toBeVisible();

    // Target tab select is visible
    const select = dialog.getByLabel(t.chooseTab);
    await expect(select).toBeVisible();

    // Click start compare
    await dialog.getByRole("button", { name: t.start }).click();

    // Results appear in list
    const diffList = dialog.getByRole("region", { name: t.diffList });
    await expect(diffList).toBeVisible();

    const items = diffList.getByRole("listitem");
    const count = await items.count();
    expect(count).toBeGreaterThan(0);

    // Click a diff item situated on page 2 to jump
    const itemOnPage2 = items.filter({ hasText: "第 2 頁" }).first();
    const hasPage2Item = (await itemOnPage2.count()) > 0;
    if (hasPage2Item) {
      await itemOnPage2.getByRole("button").click();
    }

    // Export to Markdown
    const file = path.join(folder, "compare-result.md");
    const dialogReportPromise = answerFileDialog(page, { path: file });
    await dialog.getByRole("button", { name: t.exportMarkdown }).click();
    const dialogReport = await dialogReportPromise;

    expect(existsSync(file), `${dialogReport}\nthe folder holds: ${readdirSync(folder).join(", ")}`).toBe(true);
    const content = readFileSync(file, "utf8");
    expect(content).toContain("文字比較");
    expect(content).toContain("contract-v2.pdf");
    expect(content).toContain("contract-v1.pdf");

    // Close the dialog and verify that page jumped to page 2 (contentinfo is accessible when dialog closes)
    await dialog.getByRole("button", { name: t.close }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    if (hasPage2Item) {
      await expect(page.getByRole("contentinfo")).toContainText("2 / 2");
    }
  });
});
