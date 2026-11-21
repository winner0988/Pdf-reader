import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test("the mail link confirmation names every recipient's domain (#610)", async ({ launch }) => {
  const page = await launch(corpus("malicious/link-mailto-recipients.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  const link = page.locator("button[data-link]").first();
  await link.click();

  const confirmDialog = page.getByRole("dialog", { name: strings.links.confirmTitle });
  await expect(confirmDialog).toBeVisible();

  await expect(confirmDialog.locator("[data-host]")).toHaveText("first.example, second.example");

  await page.keyboard.press("Escape");
  await expect(confirmDialog).toHaveCount(0);
});
