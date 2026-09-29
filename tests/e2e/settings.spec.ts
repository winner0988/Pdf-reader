// Settings (B2-12) in the real app: kept by the main process in the app's data folder, so they
// survive a restart; the page never writes a file itself.
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, dataDir, expect, quit, test } from "./app";

const t = strings.settings;

test("settings survive a restart, and a closed recent files list records nothing", async ({ launch }) => {
  const page = await launch();
  // With the system in light mode, only the saved setting can make the app dark.
  await page.emulateMedia({ colorScheme: "light" });
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: strings.menu.settings }).click();
  const dialog = page.getByRole("dialog", { name: t.title });
  await dialog.getByRole("radio", { name: t.themes.dark }).check();
  await expect(page.locator("html")).toHaveClass(/\bdark\b/);
  await dialog.getByRole("checkbox", { name: new RegExp(t.record) }).uncheck();

  // The main process wrote them.
  const folder = dataDir(page);
  const settingsFile = path.join(folder, "settings.json");
  await expect
    .poll(() => (existsSync(settingsFile) ? JSON.parse(readFileSync(settingsFile, "utf8")) : null))
    .toEqual({ version: 1, theme: "dark", recordRecentFiles: false });

  // Ended at once, with nothing saved on the way out; started again with the same data folder.
  await quit(page);
  const again = await launch(corpus("benign/single-page.pdf"), { dataDir: folder });
  await again.emulateMedia({ colorScheme: "light" });
  await expect(again.locator("html")).toHaveClass(/\bdark\b/);
  await expect(again.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  // Recording is off: the file that opened is not on the list, and there is no list at all.
  expect(existsSync(path.join(folder, "recent.json"))).toBe(false);
});
