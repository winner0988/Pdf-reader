// The CJK fallback font inside the sandboxed worker (DEC-03, #31). Without a substitute for a
// Chinese font the PDF does not embed, MuPDF cannot load the font: the text is neither drawn
// nor extracted, so finding it with search shows that the worker's bundled font was used.
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test("finds Chinese text in a font the PDF does not embed", async ({ launch }) => {
  const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  await page.keyboard.press("Control+f");
  const search = page.getByRole("search", { name: strings.search.label });
  await search.getByRole("textbox", { name: strings.search.placeholder }).fill("隱私優先");
  await expect(search.getByRole("status")).toHaveText(strings.search.count(1, 1));
});
