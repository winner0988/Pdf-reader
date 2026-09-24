// First end-to-end tests (QA-02): starting, opening from the command line, damaged files.
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test("starts with the empty state", async ({ launch }) => {
  const page = await launch();

  await expect(page.getByRole("heading", { name: strings.empty.title })).toBeVisible();
  await expect(page.getByText(strings.empty.privacyNote)).toBeVisible();
  await expect(page.getByRole("button", { name: strings.empty.openButton })).toBeVisible();
});

test("opens a PDF given on the command line and shows its pages", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));

  const status = page.getByRole("contentinfo");
  await expect(status).toContainText("multi-page-10.pdf");
  await expect(status).toContainText(strings.statusBar.pageStatus(1, 10, strings.toolbar.fitWidth));
  // The first page is rendered by the sandboxed worker, not just laid out.
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  // Nothing in this file is blocked.
  await expect(page.getByRole("region", { name: strings.banner.label })).toHaveCount(0);
});

test("a damaged PDF shows the error state", async ({ launch }) => {
  const page = await launch(corpus("malformed/page-tree-cycle.pdf"));

  const alert = page.getByRole("alert");
  await expect(alert).toContainText(strings.error.messages.corrupted);
  await expect(alert.getByRole("button", { name: strings.error.openAnother })).toBeVisible();
});

test("a file that is not a PDF says so", async ({ launch }) => {
  const page = await launch(corpus("malformed/not-a-pdf.pdf"));

  await expect(page.getByRole("alert")).toContainText(strings.error.messages.notPdf);
});
