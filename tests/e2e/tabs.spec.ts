// Tabs (MVP-14): launching the app again, as Explorer does for a PDF, adds the file to the running
// window as a tab, and every tab keeps its own document and view.
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
