// Encrypting a copy (B2-15) in the real app: the dialog asks for the passwords and restrictions,
// the system's save dialog (answered through UI Automation) says where the copy goes; the copy
// asks for its password when it opens and has the restrictions it was given, and the document's own
// file is not touched. A signed document cannot be encrypted: its signatures would not hold.
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const t = strings.encryption;
const OPEN = "pw-open-7Qz";
const OWNER = "pw-owner-7Qz";

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) }).first();
const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

async function openDialog(page: Page) {
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: new RegExp(strings.menu.encryptCopy.replace("…", "")) }).click();
  return page.getByRole("dialog", { name: t.title });
}

test.describe("encrypting a copy", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-encrypt-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("the copy asks for its password, has its restrictions, and the document's file is untouched", async ({
    launch,
  }) => {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
    const before = sha256(file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    const dialog = await openDialog(page);
    // Nothing is sent until something is asked for, and the two entries of a password agree.
    const start = dialog.getByRole("button", { name: t.start });
    await expect(start).toBeDisabled();
    await dialog.getByLabel(t.openPassword, { exact: true }).fill(OPEN);
    await expect(dialog.getByRole("status")).toContainText(t.problem.openMismatch);
    await dialog.getByLabel(`${t.openPassword}（${t.again}）`, { exact: true }).fill(OPEN);
    await dialog.getByRole("checkbox", { name: t.restrict.copy }).check();
    await expect(dialog.getByRole("status")).toContainText(t.problem.permissionsNeeded);
    await dialog.getByLabel(t.permissionsPassword, { exact: true }).fill(OWNER);
    await dialog.getByLabel(`${t.permissionsPassword}（${t.again}）`, { exact: true }).fill(OWNER);
    await expect(start).toBeEnabled();
    await start.click();

    const copy = path.join(folder, "secret.pdf");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(t.done);
    await expect(dialog).toHaveCount(0);

    // The document's file is as it was; the copy is AES-256 and has neither password in it.
    expect(sha256(file)).toBe(before);
    const bytes = readFileSync(copy);
    expect(bytes.includes("/AESV3")).toBe(true);
    expect(bytes.includes(OPEN) || bytes.includes(OWNER)).toBe(false);
    await quit(page);

    const reopened = await launch(copy);
    const field = reopened.getByLabel(strings.password.label, { exact: true });
    await expect(reopened.getByRole("heading", { name: strings.password.title })).toBeVisible();
    await field.fill("not the password");
    await field.press("Enter");
    await expect(reopened.getByRole("alert")).toHaveText(strings.password.wrong);
    await field.fill(OPEN);
    await field.press("Enter");
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    // What it was asked to restrict is restricted, and nothing else.
    const status = reopened.getByRole("contentinfo");
    await expect(status).toContainText(strings.permissions.noCopy);
    await expect(status).not.toContainText(strings.permissions.noPrint);
  });

  test("a signed document cannot be encrypted, and nothing is written", async ({ launch }) => {
    const page = await launch(corpus("benign/signed.pdf"));
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    const dialog = await openDialog(page);
    await dialog.getByLabel(t.openPassword, { exact: true }).fill(OPEN);
    await dialog.getByLabel(`${t.openPassword}（${t.again}）`, { exact: true }).fill(OPEN);
    await dialog.getByRole("button", { name: t.start }).click();
    const copy = path.join(folder, "signed-copy.pdf");
    await answerFileDialog(page, { path: copy });
    await expect(dialog.getByRole("alert")).toContainText(t.failed);
    expect(existsSync(copy)).toBe(false);
  });

  test("an encrypted document is not offered an encrypted copy", async ({ launch }) => {
    const page = await launch(corpus("benign/encrypted-aes256.pdf"));
    const field = page.getByLabel(strings.password.label, { exact: true });
    await field.fill("user");
    await field.press("Enter");
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    const item = page.getByRole("menuitem", { name: new RegExp(strings.menu.encryptCopy.replace("…", "")) });
    await expect(item).toBeDisabled();
    await expect(item).toContainText(t.encrypted);
  });
});
