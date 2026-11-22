// Decrypting a copy (docs/architecture/decrypt-copy.md) in the real app:
// The dialog asks for the password (owner password required if restricted);
// the system's save dialog (answered through UI Automation) says where the copy goes;
// the copy opens with no password and no restrictions, and the document's own file is not touched.
// An unencrypted document does not offer a decrypted copy.

import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const t = strings.decryption;
const USER = "user";
const OWNER = "owner";

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) }).first();
const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

async function openDialog(page: Page) {
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: new RegExp(strings.menu.decryptCopy.replace("…", "")) }).click();
  return page.getByRole("dialog", { name: t.title });
}

test.describe("saving a copy without its password", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-decrypt-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("the copy opens without password or restrictions, and the document's file is untouched", async ({
    launch,
  }) => {
    const file = path.join(folder, "restricted.pdf");
    copyFileSync(corpus("benign/restricted-open-password.pdf"), file);
    const before = sha256(file);
    const page = await launch(file);

    // Enter open password to unlock the document
    const passwordField = page.getByLabel(strings.password.label, { exact: true });
    await passwordField.fill(USER);
    await passwordField.press("Enter");
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    const dialog = await openDialog(page);
    const start = dialog.getByRole("button", { name: t.start });
    const field = dialog.getByLabel(t.password, { exact: true });

    // Submit disabled until password is typed
    await expect(start).toBeDisabled();

    // Entering user password on a restricted document is rejected
    await field.fill(USER);
    await expect(start).toBeEnabled();
    await start.click();
    const copy = path.join(folder, "unrestricted.pdf");
    await answerFileDialog(page, { path: copy });
    await expect(dialog.getByRole("alert")).toContainText(t.notAllowed);
    await expect(field).toHaveValue("");
    expect(existsSync(copy)).toBe(false);

    // Entering owner password succeeds and saves
    await field.fill(OWNER);
    await start.click();
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(t.done);
    await expect(dialog).toHaveCount(0);

    // The document's file is untouched; copy has no /Encrypt
    expect(sha256(file)).toBe(before);
    const bytes = readFileSync(copy);
    expect(bytes.includes("/Encrypt")).toBe(false);
    await quit(page);

    // Reopen copy: opens without password prompt, no restrictions
    const reopened = await launch(copy);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    const status = reopened.getByRole("contentinfo");
    await expect(status).not.toContainText(strings.permissions.notAllowed);
  });

  test("an unencrypted document is not offered a copy without password", async ({ launch }) => {
    const page = await launch(corpus("benign/multi-page-10.pdf"));
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    const item = page.getByRole("menuitem", { name: new RegExp(strings.menu.decryptCopy.replace("…", "")) });
    await expect(item).toBeDisabled();
    await expect(item).toContainText(t.unencryptedBlocked);
  });
});
