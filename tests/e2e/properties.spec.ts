// Document properties dialog (B5-04):
// Shows basic metadata, security summary and discovered fonts.
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

const t = strings.properties;

test.describe("document properties dialog (B5-04)", () => {
  test("opens via toolbar more menu and displays basic info, security, and fonts", async ({ launch }) => {
    const file = corpus("benign/multi-page-10.pdf");
    const page = await launch(file);

    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );

    // Open "⋯" menu and select "文件內容…"
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.documentProperties}`) }).click();

    // Dialog should be visible
    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toBeVisible();

    // General tab is open by default: check file name and page count
    await expect(dialog.getByText("multi-page-10.pdf")).toBeVisible();
    await expect(dialog.getByText(t.pagesCount(10))).toBeVisible();

    // Switch to Security tab
    await dialog.getByRole("tab", { name: t.tabSecurity }).click();
    await expect(dialog.getByText(t.notEncrypted)).toBeVisible();
    await expect(dialog.getByText(t.permPrint)).toBeVisible();
    await expect(dialog.getByText(t.permCopy)).toBeVisible();

    // Switch to Fonts tab
    await dialog.getByRole("tab", { name: t.tabFonts }).click();
    // In multi-page-10.pdf, fonts or empty notice should be present
    await expect(
      dialog.getByRole("table").or(dialog.getByText(t.noFonts)),
    ).toBeVisible();

    // Close dialog
    await dialog.getByRole("button", { name: t.close }).first().click();
    await expect(dialog).not.toBeVisible();
  });
});
