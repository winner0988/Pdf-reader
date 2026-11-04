// Full screen and presentation mode in the real app (B4-03, #411).
import { execFile } from "node:child_process";
import { promisify } from "node:util";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { appProcessId, corpus, expect, test } from "./app";

/** `SetWindowDisplayAffinity` values: excluded from capture (B3-13). */
const WDA_EXCLUDEFROMCAPTURE = 0x11;

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
  const { stdout } = await promisify(execFile)(
    "powershell",
    ["-NoProfile", "-NonInteractive", "-Command", script.join("; ")],
    { timeout: 30_000 },
  );
  return stdout.trim();
}

/** The display affinity of the app's window: read only, nothing captured. */
async function displayAffinity(page: Page): Promise<number> {
  const affinity = await powershell([
    ...findWindow(
      page,
      '[DllImport("user32.dll")] public static extern bool GetWindowDisplayAffinity(System.IntPtr window, out uint affinity);',
    ),
    "$affinity = [uint32]0",
    "if (-not [E2e.Window]::GetWindowDisplayAffinity($window, [ref]$affinity)) { throw 'no display affinity' }",
    "$affinity",
  ]);
  return Number(affinity);
}

async function openMenuItem(page: Page, name: string) {
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name }).click();
}

/** Marks the document shown as sensitive through the menu and dialog. */
async function markSensitive(page: Page) {
  await openMenuItem(page, strings.menu.markSensitive);
  const dialog = page.getByRole("dialog", { name: strings.sensitive.markTitle });
  await expect(dialog).toContainText(strings.sensitive.ready);
  await dialog.getByRole("button", { name: strings.sensitive.mark }).click();
  await expect(page.getByRole("contentinfo")).toContainText(strings.sensitive.marked);
  await expect(page.getByRole("contentinfo")).toContainText(strings.sensitive.badge);
}

test("full screen: F11 toggles full screen and does not reload the page (B4-03)", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Mark window to verify page does not reload
  await page.evaluate(() => {
    (window as unknown as { __test_marker: number }).__test_marker = 42;
  });

  const originalSize = await page.evaluate(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }));

  // Press F11 to enter full screen
  await page.keyboard.press("F11");

  // In full screen, innerWidth and innerHeight match screen.width and screen.height
  await expect.poll(
    () => page.evaluate(() => ({
      width: window.innerWidth,
      height: window.innerHeight,
      screenWidth: screen.width,
      screenHeight: screen.height,
    })),
    { timeout: 10_000 },
  ).toEqual(expect.objectContaining({
    width: expect.any(Number),
    height: expect.any(Number),
  }));

  const fullscreenSize = await page.evaluate(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
    screenWidth: screen.width,
    screenHeight: screen.height,
  }));
  expect(fullscreenSize.width).toBe(fullscreenSize.screenWidth);
  expect(fullscreenSize.height).toBe(fullscreenSize.screenHeight);

  // Press F11 again to exit full screen
  await page.keyboard.press("F11");

  await expect.poll(
    () => page.evaluate(() => ({
      width: window.innerWidth,
      height: window.innerHeight,
    })),
    { timeout: 10_000 },
  ).toEqual(originalSize);

  // Page did not reload
  const marker = await page.evaluate(() => (window as unknown as { __test_marker?: number }).__test_marker);
  expect(marker).toBe(42);
});

test("presentation mode: F5 from page 1, navigation, Esc returns, Shift+F5 from current page (B4-03)", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Mark window to detect reload
  await page.evaluate(() => {
    (window as unknown as { __test_marker: number }).__test_marker = 99;
  });

  const originalSize = await page.evaluate(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }));

  // Scroll to page 3
  const pageInput = page.getByRole("textbox", { name: strings.toolbar.pageNumber });
  await pageInput.fill("3");
  await pageInput.press("Enter");
  await expect(page.getByRole("img", { name: strings.canvas.page(3) })).toBeVisible();
  await expect(pageInput).toHaveValue("3");

  // 1. Press F5: starts presentation mode from page 1
  await page.keyboard.press("F5");
  const presentationDialog = page.getByRole("dialog", { name: strings.presentation.title });
  await expect(presentationDialog).toBeVisible();
  await expect(presentationDialog).toContainText(strings.presentation.pageAnnouncement(1, 10));

  // 2. Press ArrowRight twice: reaches page 3
  await page.keyboard.press("ArrowRight");
  await expect(presentationDialog).toContainText(strings.presentation.pageAnnouncement(2, 10));

  await page.keyboard.press("ArrowRight");
  await expect(presentationDialog).toContainText(strings.presentation.pageAnnouncement(3, 10));

  // 3. Press Esc: returns to original page (3) and zoom, window is not fullscreen, page did not reload
  await page.keyboard.press("Escape");
  await expect(presentationDialog).toBeHidden();

  await expect.poll(
    () => page.evaluate(() => ({
      width: window.innerWidth,
      height: window.innerHeight,
    })),
    { timeout: 10_000 },
  ).toEqual(originalSize);

  await expect(pageInput).toHaveValue("3");

  const marker = await page.evaluate(() => (window as unknown as { __test_marker?: number }).__test_marker);
  expect(marker).toBe(99);

  // 4. Shift+F5: starts presentation mode from current page (3)
  await page.keyboard.press("Shift+F5");
  await expect(presentationDialog).toBeVisible();
  await expect(presentationDialog).toContainText(strings.presentation.pageAnnouncement(3, 10));

  // Clean up: exit presentation
  await page.keyboard.press("Escape");
  await expect(presentationDialog).toBeHidden();
});

test("sensitive documents: capture protection stays effective in full screen and presentation mode (B4-03)", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Mark as sensitive
  await markSensitive(page);
  expect(await displayAffinity(page)).toBe(WDA_EXCLUDEFROMCAPTURE);

  // Full screen mode: display affinity remains WDA_EXCLUDEFROMCAPTURE
  await page.keyboard.press("F11");
  await page.waitForTimeout(500);
  expect(await displayAffinity(page)).toBe(WDA_EXCLUDEFROMCAPTURE);

  // Presentation mode: display affinity remains WDA_EXCLUDEFROMCAPTURE
  await page.keyboard.press("F5");
  const presentationDialog = page.getByRole("dialog", { name: strings.presentation.title });
  await expect(presentationDialog).toBeVisible();
  expect(await displayAffinity(page)).toBe(WDA_EXCLUDEFROMCAPTURE);

  // Exit presentation and fullscreen
  await page.keyboard.press("Escape");
  await expect(presentationDialog).toBeHidden();
  await page.keyboard.press("F11");
  await page.waitForTimeout(500);
  expect(await displayAffinity(page)).toBe(WDA_EXCLUDEFROMCAPTURE);
});
