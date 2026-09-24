// Encrypted PDFs (MVP-16) in the real app: the tab asks for the password, says so when it is
// wrong, and opens the document with the right one.
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

for (const [file, password, text] of [
  ["benign/encrypted-aes256.pdf", "user", "AES-256"],
  ["benign/encrypted-rc4-40.pdf", "owner", "Encrypted sample"],
] as const) {
  test(`${file} opens with its password`, async ({ launch }) => {
    const page = await launch(corpus(file));
    const field = page.getByLabel(strings.password.label, { exact: true });
    await expect(page.getByRole("heading", { name: strings.password.title })).toBeVisible();
    await expect(field).toBeFocused();

    await field.fill("not the password");
    await field.press("Enter");
    await expect(page.getByRole("alert")).toHaveText(strings.password.wrong);

    await field.fill(password);
    await field.press("Enter");
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
    // The text layer is readable: search finds the sample's words.
    await page.keyboard.press("Control+f");
    await page.getByRole("textbox", { name: strings.search.placeholder }).fill(text);
    await expect(page.locator("[data-highlights] polygon[data-current]")).toHaveCount(1);
  });
}
