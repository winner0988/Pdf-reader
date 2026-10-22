// Sensitive documents (B3-05, ADR 0004, ADR 0010) in the real app. Marking says first what will
// happen to the file, then appends the app's document identifier to it (the bytes before it stay
// as they were); the mark goes with the file when it is renamed and moved; a sensitive file is
// never on the recent files list; a signed file is refused with the reason. Copying from a
// sensitive document puts the text on the clipboard with the formats that keep it out of Windows'
// clipboard history and cloud clipboard: that test replaces what is on the clipboard, so it only
// runs on a runner that is thrown away (CI), or when asked for with `E2E_CLIPBOARD=1`.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, dataDir, expect, test } from "./app";
import { findOnScreen } from "./search";

const t = strings.sensitive;

const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

/** What the recent files list has, as the main process wrote it ("" when there is none). */
function recent(page: Page): string {
  const file = path.join(dataDir(page), "recent.json");
  return existsSync(file) ? readFileSync(file, "utf8") : "";
}

async function ready(page: Page) {
  await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
}

async function openMenuItem(page: Page, name: string) {
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name }).click();
}

/** Marks the document shown as sensitive, through the menu and the dialog. */
async function mark(page: Page) {
  await openMenuItem(page, strings.menu.markSensitive);
  const dialog = page.getByRole("dialog", { name: t.markTitle });
  await expect(dialog).toContainText(t.ready);
  await dialog.getByRole("button", { name: t.mark }).click();
  await expect(page.getByRole("contentinfo")).toContainText(t.marked);
  await expect(page.getByRole("contentinfo")).toContainText(t.badge);
}

test.describe("sensitive documents", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-sensitive-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("marking appends to the file, and the mark goes with it when it is renamed and moved", async ({ launch }) => {
    const file = path.join(folder, "合約 contract.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const original = readFileSync(file);
    const page = await launch(file);
    await ready(page);
    // Like any file, it is on the recent files list until it is marked.
    await expect.poll(() => recent(page)).toContain("合約 contract.pdf");

    await mark(page);
    const marked = readFileSync(file);
    expect(marked.length).toBeGreaterThan(original.length);
    expect(marked.subarray(0, original.length).equals(original)).toBe(true);
    expect(marked.includes(Buffer.from("PdfReaderDocumentId"))).toBe(true);
    await expect.poll(() => recent(page)).not.toContain("合約 contract.pdf");
    // The list of files marked here has the name, never where the file is.
    const list = readFileSync(path.join(dataDir(page), "sensitive.json"), "utf8");
    expect(list).toContain("合約 contract.pdf");
    expect(list).not.toContain(path.basename(folder));

    // Closed, then renamed and moved into another folder: still sensitive when it opens.
    await page.keyboard.press("Control+w");
    await expect(page.getByRole("heading", { name: strings.empty.title })).toBeVisible();
    const moved = path.join(folder, "elsewhere", "renamed.pdf");
    mkdirSync(path.dirname(moved));
    renameSync(file, moved);
    await page.keyboard.press("Control+o");
    await answerFileDialog(page, { path: moved });
    await ready(page);
    await expect(page.getByRole("contentinfo")).toContainText(t.badge);
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await expect(page.getByRole("menuitem", { name: strings.menu.unmarkSensitive })).toBeVisible();
    await expect(page.getByRole("menuitemcheckbox", { name: strings.menu.dontRecord })).toHaveCount(0);
    await expect.poll(() => recent(page)).not.toContain("renamed.pdf");

    // The settings list it under its name now, as open.
    await page.getByRole("menuitem", { name: strings.menu.settings }).click();
    const settings = page.getByRole("dialog", { name: strings.settings.title });
    await expect(settings.getByRole("region", { name: t.listTitle })).toContainText("renamed.pdf");
    await expect(settings.getByRole("region", { name: t.listTitle })).toContainText(t.listOpen);
    await page.keyboard.press("Escape");

    // Taken out: appended too, and the file is one like any other again.
    const before = readFileSync(moved);
    await openMenuItem(page, strings.menu.unmarkSensitive);
    const dialog = page.getByRole("dialog", { name: t.unmarkTitle });
    await dialog.getByRole("button", { name: t.unmark }).click();
    await expect(page.getByRole("contentinfo")).toContainText(t.unmarked);
    await expect(page.getByRole("contentinfo")).not.toContainText(t.badge);
    expect(readFileSync(moved).subarray(0, before.length).equals(before)).toBe(true);
    await expect.poll(() => recent(page)).toContain("renamed.pdf");
  });

  test("a signed file is not marked, and the dialog says why", async ({ launch }) => {
    const file = path.join(folder, "signed.pdf");
    copyFileSync(corpus("benign/signed.pdf"), file);
    const before = sha256(file);
    const page = await launch(file);
    await ready(page);
    await openMenuItem(page, strings.menu.markSensitive);
    const dialog = page.getByRole("dialog", { name: t.markTitle });
    await expect(dialog).toContainText(t.signed);
    await expect(dialog.getByRole("button", { name: t.mark })).toHaveCount(0);
    await dialog.getByRole("button", { name: t.close }).click();
    expect(sha256(file)).toBe(before);
    await expect(page.getByRole("contentinfo")).not.toContainText(t.badge);
  });

  const MAY_USE_CLIPBOARD = !!process.env.CI || process.env.E2E_CLIPBOARD === "1";

  test("text copied from a sensitive document stays out of the clipboard history", async ({ launch }) => {
    test.skip(
      !MAY_USE_CLIPBOARD,
      "it replaces what is on this computer's clipboard: only on a runner that is thrown away (E2E_CLIPBOARD=1 asks for it)",
    );
    const file = path.join(folder, "secret.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const page = await launch(file);
    await ready(page);
    await mark(page);
    // The page itself must not write it.
    await page.evaluate(() => {
      const written: string[] = [];
      Object.assign(window, { written });
      navigator.clipboard.writeText = async (text: string) => {
        written.push(text);
      };
    });

    // "Privacy", found and double-clicked as in text.spec.ts.
    const box = await findOnScreen(page, "Privacy");
    await page.mouse.dblclick(box.x + box.width / 2, box.y + box.height / 2);
    await page.keyboard.press("Control+c");

    const read = () =>
      execFileSync(
        "powershell",
        [
          "-NoProfile",
          "-STA",
          "-Command",
          [
            "Add-Type -AssemblyName System.Windows.Forms",
            "$data = [System.Windows.Forms.Clipboard]::GetDataObject()",
            "$formats = $data.GetFormats() -join ','",
            "$history = $data.GetData('CanIncludeInClipboardHistory')",
            "$bytes = New-Object byte[] 4",
            "if ($history) { [void]$history.Read($bytes, 0, 4) }",
            "Write-Output ($formats + '|' + ($bytes -join ',') + '|' + [System.Windows.Forms.Clipboard]::GetText())",
          ].join("; "),
        ],
        { encoding: "utf8" },
      ).trim();
    await expect.poll(read).toContain("|Privacy");
    const [formats, history] = read().split("|");
    for (const format of [
      "CanIncludeInClipboardHistory",
      "CanUploadToCloudClipboard",
      "ExcludeClipboardContentFromMonitorProcessing",
    ]) {
      expect(formats).toContain(format);
    }
    expect(history).toBe("0,0,0,0");
    expect(await page.evaluate(() => (window as unknown as { written: string[] }).written)).toEqual([]);
  });
});
