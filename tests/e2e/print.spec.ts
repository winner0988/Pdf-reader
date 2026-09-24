// Printing (MVP-17) in the real app. The system's print dialog cannot be driven from here, so
// `window.print` is recorded instead of opening it; what it would print is checked by printing
// the same page to PDF over CDP, the way Chromium's print preview does.
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

/** Page sizes of benign/mixed-page-sizes.pdf: A4, Letter, A3 landscape, 200 x 200 pt. */
const SIZES = [
  [595, 842],
  [612, 792],
  [1191, 842],
  [200, 200],
] as const;

test("prints every page in order, each at its own shape, and nothing of the app", async ({ launch }) => {
  const page = await launch(corpus("benign/mixed-page-sizes.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  await page.evaluate(() => {
    Object.assign(window, { printed: 0 });
    window.print = () => {
      (window as unknown as { printed: number }).printed += 1;
    };
  });

  await page.keyboard.press("Control+p");
  const dialog = page.getByRole("dialog", { name: strings.print.title });
  await expect(dialog.getByRole("radio", { name: strings.print.all(4) })).toBeChecked();
  await dialog.getByRole("button", { name: strings.print.next }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { printed: number }).printed)).toBe(1);
  await expect(dialog).toBeHidden();

  const images = page.locator("[data-print-pages] img");
  await expect(images).toHaveCount(4);
  const shapes = await images.evaluateAll((all) =>
    all.map((image) => {
      const img = image as HTMLImageElement;
      return { page: img.dataset.page, ratio: img.naturalWidth / img.naturalHeight };
    }),
  );
  expect(shapes.map((shape) => shape.page)).toEqual(["1", "2", "3", "4"]);
  shapes.forEach((shape, index) => {
    const [width, height] = SIZES[index]!;
    expect(shape.ratio).toBeCloseTo(width / height, 2);
  });

  // What goes to the printer: one sheet per page, and the app itself nowhere.
  await page.emulateMedia({ media: "print" });
  await expect(page.locator("#root")).toBeHidden();
  await expect(images.first()).toBeVisible();
  const cdp = await page.context().newCDPSession(page);
  const { data } = await cdp.send("Page.printToPDF", { printBackground: false });
  const pdf = Buffer.from(data, "base64").toString("latin1");
  expect(pdf.match(/\/Type\s*\/Page[^s]/g)).toHaveLength(4);
});
