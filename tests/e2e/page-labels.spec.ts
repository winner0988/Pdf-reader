// Page labels (B5-07): display document's own labels and accept them in the page input.
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test("displays page labels and jumps when typing a label", async ({ launch }) => {
  const page = await launch(corpus("benign/page-labels-styles.pdf"));
  await page.setViewportSize({ width: 1024, height: 700 });
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  const pageInput = page.getByRole("textbox", { name: strings.toolbar.pageNumberWithLabel("i", 1, 10) });
  await expect(pageInput).toHaveValue("i");

  // Jump by prefixed label 'A-1' (page 9)
  await pageInput.fill("A-1");
  await pageInput.press("Enter");

  const pageInput9 = page.getByRole("textbox", { name: strings.toolbar.pageNumberWithLabel("A-1", 9, 10) });
  await expect(pageInput9).toHaveValue("A-1");
  await expect(page.getByRole("contentinfo")).toContainText(strings.statusBar.pageStatus(9, 10, strings.toolbar.fitWidth));

  // Jump by label 'iv' (page 4)
  await pageInput9.fill("iv");
  await pageInput9.press("Enter");

  const pageInput4 = page.getByRole("textbox", { name: strings.toolbar.pageNumberWithLabel("iv", 4, 10) });
  await expect(pageInput4).toHaveValue("iv");
  await expect(page.getByRole("contentinfo")).toContainText(strings.statusBar.pageStatus(4, 10, strings.toolbar.fitWidth));

  // Check thumbnails on page 4
  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  const thumbnails = page.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
  const thumb4 = thumbnails.getByRole("option", { name: strings.canvas.pageWithLabel("iv", 4, 10) });
  await expect(thumb4).toBeVisible();
  await expect(thumb4).toHaveAttribute("aria-current", "page");
  await expect(thumb4).toContainText(strings.pages.thumbnailLabel("iv", 4, 10));

  // Delete page 4 (label 'iv') from thumbnails; page labels update to match 9 pages
  await thumb4.click({ button: "right" });
  await page.getByRole("menuitem", { name: new RegExp(`^${strings.pages.delete}`) }).click();
  await expect(page.getByRole("menu")).toHaveCount(0);

  // Toolbar page input now has label '1' and shows count (4 / 9)
  const pageInput4After = page.getByRole("textbox", { name: strings.toolbar.pageNumberWithLabel("1", 4, 9) });
  await expect(pageInput4After).toBeVisible();
  await expect(pageInput4After).toHaveValue("1");
  await expect(page.getByText(strings.toolbar.pageCountWithLabel(4, 9))).toBeVisible();

  // Page 4 thumbnail now has label '1' out of 9 pages
  const newThumb4 = thumbnails.getByRole("option", { name: strings.canvas.pageWithLabel("1", 4, 9) });
  await expect(newThumb4).toBeVisible();
  await expect(newThumb4).toContainText(strings.pages.thumbnailLabel("1", 4, 9));
});
