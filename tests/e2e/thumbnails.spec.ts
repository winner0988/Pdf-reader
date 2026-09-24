// Page thumbnails (MVP-18) in the real app: rendered by the worker, and a click goes to the page.
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test("thumbnails show the pages and take the reader to one", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  const thumbnails = page.getByRole("list", { name: strings.sidebar.thumbnailsTab });
  const first = thumbnails.getByRole("button", { name: strings.canvas.page(1) });
  await expect(first).toHaveAttribute("aria-current", "page");
  // Rendered: the canvas has pixels.
  await expect.poll(() => first.locator("canvas").evaluate((canvas: HTMLCanvasElement) => canvas.width)).toBeGreaterThan(0);

  await thumbnails.getByRole("button", { name: strings.canvas.page(7) }).click();
  const status = page.getByRole("contentinfo");
  await expect(status).toContainText(strings.statusBar.pageStatus(7, 10, strings.toolbar.fitWidth));
  await expect(thumbnails.getByRole("button", { name: strings.canvas.page(7) })).toHaveAttribute("aria-current", "page");
});
