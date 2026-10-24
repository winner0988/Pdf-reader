// Sensitive documents (B3-05, ADR 0004, ADR 0010) in the real app. Marking says first what will
// happen to the file, then appends the app's document identifier to it (the bytes before it stay
// as they were); the mark goes with the file when it is renamed and moved; a sensitive file is
// never on the recent files list; a signed file is refused with the reason. Copying from a
// sensitive document puts the text on the clipboard with the formats that keep it out of Windows'
// clipboard history and cloud clipboard: that test replaces what is on the clipboard, so it only
// runs on a runner that is thrown away (CI), or when asked for with `E2E_CLIPBOARD=1`.
import { execFile, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { promisify } from "node:util";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, appProcessId, corpus, dataDir, expect, launchAgain, test } from "./app";
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

/** `SetWindowDisplayAffinity`'s values (B3-13): kept out of screen captures, or not. */
const WDA_EXCLUDEFROMCAPTURE = 0x11;
const WDA_NONE = 0;

/**
 * A PowerShell script's first lines: `$window` is the app's own window (the visible one of the
 * class Tauri gives it, as `appWindowShown` finds it), and `[E2e.Window]` has `extra` too.
 */
function findWindow(page: Page, extra: string): string[] {
  return [
    `Add-Type -Namespace E2e -Name Window -MemberDefinition '[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern System.IntPtr FindWindowEx(System.IntPtr parent, System.IntPtr after, string className, string title); [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr window, out uint process); [DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr window); ${extra}'`,
    "$window = [System.IntPtr]::Zero",
    "$found = [System.IntPtr]::Zero",
    `do { $window = [E2e.Window]::FindWindowEx([System.IntPtr]::Zero, $window, 'Tauri Window', [NullString]::Value); if ($window -ne [System.IntPtr]::Zero) { $process = [uint32]0; [void][E2e.Window]::GetWindowThreadProcessId($window, [ref]$process); if ($process -eq ${appProcessId(page)} -and [E2e.Window]::IsWindowVisible($window)) { $found = $window } } } while ($window -ne [System.IntPtr]::Zero)`,
    "if ($found -eq [System.IntPtr]::Zero) { throw 'the app shows no window' }",
    "$window = $found",
  ];
}

async function powershell(script: string[]): Promise<string> {
  const { stdout } = await promisify(execFile)("powershell", ["-NoProfile", "-NonInteractive", "-Command", script.join("; ")], {
    timeout: 30_000,
  });
  return stdout.trim();
}

/** The display affinity of the app's window, as Windows says it is: read, nothing is captured. */
async function displayAffinity(page: Page): Promise<number> {
  const affinity = await powershell([
    ...findWindow(page, '[DllImport("user32.dll")] public static extern bool GetWindowDisplayAffinity(System.IntPtr window, out uint affinity);'),
    "$affinity = [uint32]0",
    "if (-not [E2e.Window]::GetWindowDisplayAffinity($window, [ref]$affinity)) { throw 'no display affinity' }",
    "$affinity",
  ]);
  return Number(affinity);
}

/**
 * What a screen capture has where the app's window is: 32 by 32 points spread over it, each as
 * "rrggbb". It captures this computer's screen, so only a runner that is thrown away calls it.
 */
async function captureWindow(page: Page): Promise<string[]> {
  const colors = await powershell([
    "Add-Type -AssemblyName System.Drawing",
    ...findWindow(
      page,
      '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left; public int Top; public int Right; public int Bottom; } [DllImport("user32.dll")] public static extern bool GetWindowRect(System.IntPtr window, out Rect rect);',
    ),
    "[void][E2e.Window]::SetProcessDPIAware()",
    "$rect = New-Object E2e.Window+Rect",
    "[void][E2e.Window]::GetWindowRect($window, [ref]$rect)",
    "$width = $rect.Right - $rect.Left",
    "$height = $rect.Bottom - $rect.Top",
    "$bitmap = New-Object System.Drawing.Bitmap $width, $height",
    "$graphics = [System.Drawing.Graphics]::FromImage($bitmap)",
    "$graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)",
    "$colors = for ($y = 0; $y -lt 32; $y++) { for ($x = 0; $x -lt 32; $x++) { $color = $bitmap.GetPixel([int](($x + 0.5) * $width / 32), [int](($y + 0.5) * $height / 32)); '{0:x2}{1:x2}{2:x2}' -f $color.R, $color.G, $color.B } }",
    "$colors -join ','",
  ]);
  return colors.split(",");
}

/** The share of the points two captures have nearly the same color at. */
function sameShare(a: string[], b: string[]): number {
  const close = (x: string, y: string) =>
    [0, 2, 4].every((at) => Math.abs(parseInt(x.slice(at, at + 2), 16) - parseInt(y.slice(at, at + 2), 16)) <= 16);
  return a.filter((color, index) => close(color, b[index] ?? "")).length / a.length;
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

  test("while a sensitive document is open, the window is kept out of screen captures, every tab of it (B3-13)", async ({
    launch,
  }) => {
    const secret = path.join(folder, "secret.pdf");
    const plain = path.join(folder, "plain.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), secret);
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), plain);
    const page = await launch(secret);
    await ready(page);
    const status = page.getByRole("contentinfo");
    expect(await displayAffinity(page)).toBe(WDA_NONE);
    await expect(status).not.toContainText(t.captureOn);

    // Marked: kept out at once, and the status bar says so.
    await mark(page);
    await expect(status).toContainText(t.captureOn);
    expect(await displayAffinity(page)).toBe(WDA_EXCLUDEFROMCAPTURE);

    // Closed: let back in.
    await page.keyboard.press("Control+w");
    await expect(page.getByRole("heading", { name: strings.empty.title })).toBeVisible();
    await expect.poll(() => displayAffinity(page)).toBe(WDA_NONE);

    // Opened again: kept out by the time its first page is shown (the main process changes the
    // window before it tells the page of the document).
    launchAgain(secret);
    await ready(page);
    expect(await displayAffinity(page)).toBe(WDA_EXCLUDEFROMCAPTURE);
    await expect(status).toContainText(t.badge);
    await expect(status).toContainText(t.captureOn);

    // Another document in another tab: the whole window is kept out, so it says so there too.
    launchAgain(plain);
    await expect(page.getByRole("tab", { name: "plain.pdf" })).toHaveAttribute("aria-selected", "true");
    await ready(page);
    await expect(status).not.toContainText(t.badge);
    await expect(status).toContainText(t.captureOn);
    expect(await displayAffinity(page)).toBe(WDA_EXCLUDEFROMCAPTURE);

    // The only sensitive document unmarked: let back in, though both tabs stay open.
    await page.getByRole("tab", { name: "secret.pdf" }).click();
    await openMenuItem(page, strings.menu.unmarkSensitive);
    await page.getByRole("dialog", { name: t.unmarkTitle }).getByRole("button", { name: t.unmark }).click();
    await expect(status).toContainText(t.unmarked);
    await expect(status).not.toContainText(t.captureOn);
    expect(await displayAffinity(page)).toBe(WDA_NONE);
  });

  const MAY_CAPTURE_SCREEN = !!process.env.CI || process.env.E2E_CAPTURE === "1";

  test("a screen capture gets nothing of the window while a sensitive document is open (B3-13)", async ({ launch }) => {
    test.skip(
      !MAY_CAPTURE_SCREEN,
      "it captures this computer's screen: only on a runner that is thrown away (E2E_CAPTURE=1 asks for it)",
    );
    const file = path.join(folder, "secret.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const page = await launch(file);
    await ready(page);
    const shown = await captureWindow(page);
    // The capture has the window: much of it is the white page.
    expect(shown.filter((color) => color === "ffffff").length / shown.length).toBeGreaterThan(0.2);

    await mark(page);
    await expect(page.getByRole("contentinfo")).toContainText(t.captureOn);
    const kept = await captureWindow(page);
    expect(sameShare(shown, kept)).toBeLessThan(0.5);

    await openMenuItem(page, strings.menu.unmarkSensitive);
    await page.getByRole("dialog", { name: t.unmarkTitle }).getByRole("button", { name: t.unmark }).click();
    await expect(page.getByRole("contentinfo")).not.toContainText(t.captureOn);
    await expect.poll(async () => sameShare(shown, await captureWindow(page))).toBeGreaterThan(0.9);
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
