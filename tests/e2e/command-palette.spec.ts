// Command palette (Ctrl+K) walkthrough (B5-06, Issue #443).
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test.describe("command palette (Ctrl+K)", () => {
  test("finds and executes a command on an open document", async ({ launch }) => {
    const page = await launch(corpus("benign/single-page.pdf"));
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toBeVisible();

    // Open palette with Ctrl+K
    await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: strings.commands.palette });
    await expect(palette).toBeVisible();

    const input = palette.getByRole("combobox");
    await expect(input).toBeFocused();

    // Type "符合頁面" and press Enter to execute zoom command
    await page.keyboard.type("符合頁面");
    const option = palette.getByRole("option", { name: /符合頁面/ });
    await expect(option).toBeVisible();
    await page.keyboard.press("Enter");

    // Palette closes and view updates
    await expect(palette).not.toBeVisible();
    await expect(page.getByRole("contentinfo")).toContainText(strings.toolbar.fitPage);
  });

  test("shows disabled reason for restricted command and does not execute", async ({ launch }) => {
    const page = await launch(corpus("benign/restricted-no-copy-no-print.pdf"));
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toBeVisible();

    await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: strings.commands.palette });
    await expect(palette).toBeVisible();

    // Search for printing command which is blocked by author permissions
    await page.keyboard.type("列印");
    const printOption = palette.getByRole("option", { name: new RegExp(`^${strings.menu.print}`) });
    await expect(printOption).toBeVisible();
    await expect(printOption).toHaveAttribute("aria-disabled", "true");
    await expect(printOption).toContainText(strings.permissions.notAllowed);

    // Pressing Enter does not close the dialog because command is disabled
    await page.keyboard.press("Enter");
    await expect(palette).toBeVisible();

    // Dismiss with Escape
    await page.keyboard.press("Escape");
    await expect(palette).not.toBeVisible();
  });
});
