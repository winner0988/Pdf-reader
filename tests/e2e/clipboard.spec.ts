// Copying text (MVP-10, #209) puts it on the clipboard with `navigator.clipboard.writeText`, and
// ignores a failure without a word. The other tests replace `writeText` with a recorder, so that
// they never touch this computer's clipboard; this one asks the real WebView what the page may
// do, and writes nothing: it may write (that is all copying needs) and may not read (a page that
// could read the clipboard unasked would be a leak).
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test("the page may write to the clipboard, which is what copying needs, and may not read it (#209)", async ({
  launch,
}) => {
  const page = await launch(corpus("benign/single-page.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");

  const facts = await page.evaluate(async () => {
    const state = async (name: string) => (await navigator.permissions.query({ name: name as PermissionName })).state;
    return {
      secure: window.isSecureContext,
      writeText: typeof navigator.clipboard?.writeText,
      write: await state("clipboard-write"),
      read: await state("clipboard-read"),
    };
  });

  expect(facts.secure).toBe(true);
  expect(facts.writeText).toBe("function");
  expect(facts.write).toBe("granted");
  // Asked for, never granted by itself: the user would be asked.
  expect(facts.read).not.toBe("granted");
});
