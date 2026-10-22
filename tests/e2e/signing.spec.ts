// Signing a document (B3-06, ADR 0020) in the real app: the certificate file is chosen in the
// system's open dialog and the signed copy's place in its save dialog (both answered through UI
// Automation); the main process signs with the corpus's test certificate, a new worker verifies the
// copy, and the copy opens in a tab of its own with its signature valid. The document's own file is
// not touched, and the password shows nowhere.
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

type TauriWindow = {
  __TAURI_INTERNALS__: { invoke(command: string, args?: Record<string, unknown>): Promise<unknown> };
};

const t = strings.signing;
const PASSWORD = "corpus-test-password";
const SIGNER = "PDF Reader test corpus signer (NOT TRUSTED)";

/**
 * Importing a certificate file adds it to Windows' own key machinery for the current user: only on
 * a runner that is thrown away after the run (CI), or when asked for with `E2E_SIGNING=1`.
 */
const MAY_IMPORT = !!process.env.CI || process.env.E2E_SIGNING === "1";
const WHY_NOT = "it imports a test certificate: only on a runner that is thrown away (E2E_SIGNING=1 asks for it)";

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) }).first();
const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

async function openDialog(page: Page) {
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: new RegExp(strings.menu.sign.replace("…", "")) }).click();
  return page.getByRole("dialog", { name: t.title });
}

async function chooseCertificate(page: Page, name: string) {
  const dialog = page.getByRole("dialog", { name: t.title });
  await dialog.getByRole("button", { name: t.chooseFile }).click();
  await answerFileDialog(page, { path: corpus(`certificates/${name}`) });
  await expect(dialog).toContainText(name);
}

test.describe("signing a document", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-signing-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("the signed copy opens with its signature valid, and the document's file is untouched", async ({ launch }) => {
    test.skip(!MAY_IMPORT, WHY_NOT);
    const file = path.join(folder, "contract.pdf");
    copyFileSync(corpus("benign/single-page.pdf"), file);
    const before = sha256(file);
    const original = readFileSync(file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    const dialog = await openDialog(page);
    const start = dialog.getByRole("button", { name: t.start });
    await expect(start).toBeDisabled();
    await chooseCertificate(page, "signer.pfx");
    await dialog.getByLabel(t.passwordLabel, { exact: true }).fill(PASSWORD);
    await dialog.getByLabel(t.reasonLabel, { exact: true }).fill("同意本合約");
    await start.click();

    const copy = path.join(folder, "contract（已簽署）.pdf");
    await answerFileDialog(page, { path: copy });
    // Signed, the dialog says so in its title: it is found by that one.
    const done = page.getByRole("dialog", { name: t.resultTitle });
    await expect(done.getByRole("status")).toContainText(t.result("contract（已簽署）.pdf"));
    await done.getByRole("button", { name: t.done }).click();

    // The copy opened in a tab of its own, and its signature holds; nobody vouches for a test signer.
    await expect(page.getByRole("tab", { name: /contract（已簽署）\.pdf/ })).toHaveAttribute("aria-selected", "true");
    const banner = page.getByRole("region", { name: strings.signatures.label });
    await expect(banner).toHaveAttribute("data-signatures", "unconfirmed");
    await banner.getByRole("button", { name: strings.signatures.details }).click();
    const panel = page.getByRole("complementary", { name: strings.signatures.detailsTitle });
    await expect(panel).toContainText(strings.signatures.status.unconfirmed);
    await expect(panel).toContainText(SIGNER);

    // The document's file is as it was; the copy is that file with the signature appended, and has
    // no password in it.
    expect(sha256(file)).toBe(before);
    const signed = readFileSync(copy);
    expect(signed.subarray(0, original.length).equals(original)).toBe(true);
    expect(signed.includes(PASSWORD)).toBe(false);
    expect(await page.content()).not.toContain(PASSWORD);
  });

  test("a wrong password says only that, and nothing is written", async ({ launch }) => {
    test.skip(!MAY_IMPORT, WHY_NOT);
    const page = await launch(corpus("benign/single-page.pdf"));
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    const dialog = await openDialog(page);
    await chooseCertificate(page, "signer.pfx");
    const field = dialog.getByLabel(t.passwordLabel, { exact: true });
    await field.fill("not the password");
    await dialog.getByRole("button", { name: t.start }).click();
    // Asked before the save dialog: no file is picked, and none is written.
    await expect(dialog.getByRole("alert")).toHaveText(strings.error.messages.certificateUnreadable);
    await expect(field).toHaveValue("");
    expect(existsSync(path.join(folder, "single-page（已簽署）.pdf"))).toBe(false);

    // An expired certificate is refused too, with its reason.
    await chooseCertificate(page, "expired.pfx");
    await field.fill(PASSWORD);
    await dialog.getByRole("button", { name: t.start }).click();
    await expect(dialog.getByRole("alert")).toHaveText(strings.error.messages.certificateExpired);
  });

  test("a document with changes not saved is not signed: the file would not have them", async ({ launch }) => {
    const page = await launch(corpus("benign/multi-page-10.pdf"));
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    // Page 1 turned through the app's IPC, as saving.spec.ts does: a change not in the file.
    const doc = Number(await firstPage(page).getAttribute("data-doc"));
    await page.evaluate(async (doc) => {
      await (window as unknown as TauriWindow).__TAURI_INTERNALS__.invoke("apply_edit", {
        args: { doc, edit: { kind: "rotatePages", pages: [0], by: "cw90" } },
      });
    }, doc);
    await expect(page.getByRole("tab", { name: new RegExp(`${strings.tabs.unsaved}$`) })).toBeVisible();

    const dialog = await openDialog(page);
    await expect(dialog.getByRole("alert")).toHaveText(t.unsaved);
    await expect(dialog.getByRole("button", { name: t.start })).toHaveCount(0);
  });

  test("an encrypted document is not offered signing", async ({ launch }) => {
    const page = await launch(corpus("benign/encrypted-aes256.pdf"));
    const field = page.getByLabel(strings.password.label, { exact: true });
    await field.fill("user");
    await field.press("Enter");
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    const item = page.getByRole("menuitem", { name: new RegExp(strings.menu.sign.replace("…", "")) });
    await expect(item).toBeDisabled();
    await expect(item).toContainText(t.encryptedShort);
  });
});
