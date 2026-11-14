// Page labels (B5-07): display document's own labels and accept them in the page input.
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test("displays page labels and jumps when typing a label", async ({ launch }) => {
  const page = await launch(corpus("benign/page-labels.pdf"));
  await page.setViewportSize({ width: 1024, height: 700 });
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  const pageInput = page.getByRole("textbox", { name: strings.toolbar.pageNumberWithLabel("i", 1, 10) });
  await expect(pageInput).toHaveValue("i");

  // Jump by label 'iv' (page 4)
  await pageInput.fill("iv");
  await pageInput.press("Enter");

  const pageInput4 = page.getByRole("textbox", { name: strings.toolbar.pageNumberWithLabel("iv", 4, 10) });
  await expect(pageInput4).toHaveValue("iv");
  await expect(page.getByRole("contentinfo")).toContainText(strings.statusBar.pageStatus(4, 10, strings.toolbar.fitWidth));

  // Check thumbnails
  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  const thumbnails = page.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
  const thumb4 = thumbnails.getByRole("option", { name: strings.canvas.pageWithLabel("iv", 4, 10) });
  await expect(thumb4).toBeVisible();
  await expect(thumb4).toHaveAttribute("aria-current", "page");
  await expect(thumb4).toContainText(strings.pages.thumbnailLabel("iv", 4, 10));

  // Jump by prefixed label 'A-1' (page 9)
  await pageInput4.fill("A-1");
  await pageInput4.press("Enter");

  const pageInput9 = page.getByRole("textbox", { name: strings.toolbar.pageNumberWithLabel("A-1", 9, 10) });
  await expect(pageInput9).toHaveValue("A-1");
  await expect(page.getByRole("contentinfo")).toContainText(strings.statusBar.pageStatus(9, 10, strings.toolbar.fitWidth));
});
