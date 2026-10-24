// Attachments (B3-20, ADR 0025) in the real app:
// Attachments are listed with their genuine extensions, can be saved to disk,
// receive the Mark of the Web (Zone.Identifier stream), and dangerous executable types
// require confirmation before saving.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

const t = strings.attachments;

test.describe("attachments", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-attachments-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("saving a plain text attachment preserves content and writes Zone.Identifier", async ({
    launch,
  }) => {
    const file = corpus("benign/attachment-text.pdf");
    const page = await launch(file);

    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );

    // Switch to attachments tab in sidebar
    await page.getByRole("tab", { name: strings.sidebar.attachmentsTab }).click();
    await expect(page.getByTestId("attachments-panel")).toBeVisible();
    await expect(page.getByText("sample.txt")).toBeVisible();
    await expect(page.getByTestId("attachment-extension")).toHaveText(".txt");

    // Click Save As
    const dest = path.join(folder, "sample.txt");
    await page.getByTestId("save-attachment-button").click();
    await answerFileDialog(page, { path: dest });
    await expect(page.getByTestId("attachments-notice")).toHaveText(t.saveSuccess);

    // Verify saved content
    const content = readFileSync(dest, "utf-8");
    expect(content).toBe("This is a benign text file attachment.\n");

    // Verify Mark of the Web Zone.Identifier ADS
    const zoneContent = readFileSync(`${dest}:Zone.Identifier`, "utf-8");
    expect(zoneContent).toContain("[ZoneTransfer]");
    expect(zoneContent).toContain("ZoneId=3");
  });

  test("saving dangerous attachment requires confirmation before showing file dialog", async ({
    launch,
  }) => {
    const file = corpus("malicious/attachment-exe.pdf");
    const page = await launch(file);

    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );

    // Switch to attachments tab
    await page.getByRole("tab", { name: strings.sidebar.attachmentsTab }).click();
    await expect(page.getByTestId("attachments-panel")).toBeVisible();
    await expect(page.getByText("payload.exe")).toBeVisible();
    await expect(page.getByTestId("attachment-dangerous")).toBeVisible();

    // Click Save As
    await page.getByTestId("save-attachment-button").click();

    // Danger confirmation dialog must appear
    const dialog = page.getByTestId("danger-confirm-dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(t.dangerConfirm.title);

    // Confirm save
    const dest = path.join(folder, "payload.exe");
    await dialog.getByTestId("danger-confirm-button").click();
    await answerFileDialog(page, { path: dest });
    await expect(page.getByTestId("attachments-notice")).toHaveText(t.saveSuccess);

    // Verify saved file has Zone.Identifier
    const zoneContent = readFileSync(`${dest}:Zone.Identifier`, "utf-8");
    expect(zoneContent).toContain("[ZoneTransfer]");
    expect(zoneContent).toContain("ZoneId=3");
  });
});
