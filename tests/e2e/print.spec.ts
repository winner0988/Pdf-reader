// Printing (MVP-17, B5-15) in the real app. The system's print dialog cannot be driven from here, so
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

test("prints 4 pages on one sheet with 4 cells and correct page order", async ({ launch }) => {
  const page = await launch(corpus("benign/mixed-page-sizes.pdf"));
  await expect(page.locator('[data-state="ready"]').first()).toBeVisible();
  await page.evaluate(() => {
    Object.assign(window, { printed: 0 });
    window.print = () => {
      (window as unknown as { printed: number }).printed += 1;
    };
  });

  await page.keyboard.press("Control+p");
  const dialog = page.getByRole("dialog", { name: strings.print.title });
  await expect(dialog.getByRole("radio", { name: strings.print.all(4) })).toBeChecked();

  // Select "每張 4 頁"
  await dialog.getByRole("combobox", { name: strings.print.layout }).selectOption("4");
  // Check border checkbox
  await dialog.getByRole("checkbox", { name: strings.print.pageBorder }).check();

  await dialog.getByRole("button", { name: strings.print.next }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { printed: number }).printed)).toBe(1);
  await expect(dialog).toBeHidden();

  // 1 sheet, 4 cells, in order 1, 2, 3, 4
  const sheets = page.locator("[data-print-sheet]");
  await expect(sheets).toHaveCount(1);

  const cells = sheets.first().locator("[data-print-cell]");
  await expect(cells).toHaveCount(4);

  for (let i = 0; i < 4; i++) {
    await expect(cells.nth(i)).toHaveAttribute("data-border", "true");
    await expect(cells.nth(i).locator("img")).toHaveAttribute("data-page", String(i + 1));
  }
});

test("prints 8 pages as a booklet with 4 faces and correct booklet page order", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.locator('[data-state="ready"]').first()).toBeVisible();
  await page.evaluate(() => {
    Object.assign(window, { printed: 0 });
    window.print = () => {
      (window as unknown as { printed: number }).printed += 1;
    };
  });

  await page.keyboard.press("Control+p");
  const dialog = page.getByRole("dialog", { name: strings.print.title });

  // Select page range "1-8"
  await dialog.getByRole("radio", { name: strings.print.pages }).check();
  await dialog.getByPlaceholder(strings.print.pagesPlaceholder).fill("1-8");

  // Select "小冊子"
  await dialog.getByRole("combobox", { name: strings.print.layout }).selectOption("booklet");
  await expect(dialog.getByText(strings.print.bookletHint)).toBeVisible();

  await dialog.getByRole("button", { name: strings.print.next }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { printed: number }).printed)).toBe(1);
  await expect(dialog).toBeHidden();

  // 4 faces (sheets)
  const sheets = page.locator("[data-print-sheet]");
  await expect(sheets).toHaveCount(4);

  // Check landscape style
  expect(await page.locator("[data-print-pages] style").textContent()).toContain("@page { size: landscape; }");

  // Expected booklet page order across the 4 faces:
  // Face 0: [8, 1]
  // Face 1: [2, 7]
  // Face 2: [6, 3]
  // Face 3: [4, 5]
  const expectedFaces = [
    ["8", "1"],
    ["2", "7"],
    ["6", "3"],
    ["4", "5"],
  ];

  for (let faceIdx = 0; faceIdx < 4; faceIdx++) {
    const face = sheets.nth(faceIdx);
    const cells = face.locator("[data-print-cell]");
    await expect(cells).toHaveCount(2);
    const [left, right] = expectedFaces[faceIdx]!;
    await expect(cells.nth(0).locator("img")).toHaveAttribute("data-page", left!);
    await expect(cells.nth(1).locator("img")).toHaveAttribute("data-page", right!);
  }
});
