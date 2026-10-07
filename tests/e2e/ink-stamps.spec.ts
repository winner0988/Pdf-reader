// The pen and the standard stamps (B2-08) in the real app. A line is drawn with the mouse and a
// stamp put down with a click; both are moved and the document is saved as a copy. The copy opens
// again with them: standard PDF annotations that say nothing of who made them.
import { copyFileSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Locator, Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const t = strings.annotations;

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) });
const inks = (page: Page) => page.locator("[data-page-annotations] [data-annotation=ink]");
const stamps = (page: Page) => page.locator("[data-page-annotations] [data-annotation=stamp]");
/** The layer the pen and the stamps are used on: there while a tool is on. */
const drawing = (page: Page) => page.locator("[data-page-drawing]");

type Box = { x: number; y: number; width: number; height: number };

async function boxOf(locator: Locator): Promise<Box> {
  const box = await locator.boundingBox();
  if (!box) throw new Error("it is not on screen");
  return box;
}

/**
 * Where the first of `locator` is, or `null` while it is not (an edit gives the page new
 * annotations, which are not there for a moment).
 */
const placeOf = (locator: Locator) => locator.first().boundingBox();

/** How many pixels of the first page as drawn (its canvas) inside `box` of the screen are `hue`. */
async function pixelsOf(page: Page, box: Box, hue: "red" | "blue"): Promise<number> {
  return firstPage(page)
    .locator("canvas")
    .evaluate(
      (canvas: HTMLCanvasElement, args) => {
        const bounds = canvas.getBoundingClientRect();
        const scaleX = canvas.width / bounds.width;
        const scaleY = canvas.height / bounds.height;
        const x0 = Math.max(0, Math.floor((args.box.x - bounds.left) * scaleX));
        const y0 = Math.max(0, Math.floor((args.box.y - bounds.top) * scaleY));
        const x1 = Math.min(canvas.width, Math.ceil((args.box.x + args.box.width - bounds.left) * scaleX));
        const y1 = Math.min(canvas.height, Math.ceil((args.box.y + args.box.height - bounds.top) * scaleY));
        const data = canvas.getContext("2d")!.getImageData(x0, y0, Math.max(x1 - x0, 1), Math.max(y1 - y0, 1)).data;
        let count = 0;
        for (let at = 0; at < data.length; at += 4) {
          const [red, green, blue] = [data[at]!, data[at + 1]!, data[at + 2]!];
          const match =
            args.hue === "red" ? red > 150 && green < 90 && blue < 90 : blue > 150 && red < 90 && green < 130;
          if (match) count++;
        }
        return count;
      },
      { box, hue },
    );
}

/** Presses the mouse at the first of `points`, moves through the others and lets go. */
async function draw(page: Page, points: Array<[number, number]>) {
  await page.mouse.move(...points[0]!);
  await page.mouse.down();
  for (const point of points.slice(1)) await page.mouse.move(...point, { steps: 5 });
  await page.mouse.up();
}

test("a line drawn with the pen and a stamp put down are in the saved copy, as standard annotations without an author (B2-08)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-ink-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await expect(inks(page)).toHaveCount(0);
    // The whole page on the screen. Its text is at the top: the rest is white, where the pen and
    // the stamps go.
    await page.keyboard.press("Control+0");
    await expect.poll(async () => (await boxOf(firstPage(page))).height).toBeLessThan(700);
    const sheet = await boxOf(firstPage(page));
    const at = (across: number, down: number): [number, number] => [
      sheet.x + sheet.width * across,
      sheet.y + sheet.height * down,
    ];

    // The pen, in blue (the menu next to its button closes with Esc, and the pen stays on).
    await page.getByRole("button", { name: t.pen, exact: true }).click();
    await page.getByRole("button", { name: t.penStyle }).click();
    await page.getByRole("menuitemradio", { name: t.inkColors.blue }).click();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await expect(drawing(page)).toHaveCount(1);

    await draw(page, [at(0.25, 0.35), at(0.45, 0.45), at(0.65, 0.35)]);
    await expect(inks(page)).toHaveCount(1);
    await expect(inks(page).first()).toHaveAccessibleName(t.kind.ink);
    // The line is where it was drawn, and the page shows it in blue.
    const line = await boxOf(inks(page).first());
    expect(line.x).toBeLessThanOrEqual(at(0.25, 0)[0] + 1);
    expect(line.x + line.width).toBeGreaterThanOrEqual(at(0.65, 0)[0] - 1);
    expect(line.y).toBeLessThanOrEqual(at(0, 0.35)[1] + 1);
    expect(line.y + line.height).toBeGreaterThanOrEqual(at(0, 0.45)[1] - 1);
    await expect.poll(() => pixelsOf(page, line, "blue")).toBeGreaterThan(50);

    // The pen stays on for the next line; undo takes the last one away, and Esc puts the pen away.
    await draw(page, [at(0.25, 0.55), at(0.65, 0.55)]);
    await expect(inks(page)).toHaveCount(2);
    await page.keyboard.press("Control+z");
    await expect(inks(page)).toHaveCount(1);
    await page.keyboard.press("Escape");
    await expect(drawing(page)).toHaveCount(0);

    // A stamp: chosen from the menu, put down by a click where it is to be (in the middle of it).
    await page.getByRole("button", { name: t.stamp }).click();
    await page.getByRole("menuitem", { name: t.stamps.draft }).click();
    await expect(page.getByRole("contentinfo")).toContainText(t.stamps.draft);
    const [stampX, stampY] = at(0.5, 0.75);
    await page.mouse.click(stampX, stampY);
    await expect(stamps(page)).toHaveCount(1);
    await expect(drawing(page)).toHaveCount(0);
    const stamp = await boxOf(stamps(page).first());
    expect(Math.abs(stamp.x + stamp.width / 2 - stampX)).toBeLessThan(3);
    expect(Math.abs(stamp.y + stamp.height / 2 - stampY)).toBeLessThan(3);
    await expect.poll(() => pixelsOf(page, stamp, "red")).toBeGreaterThan(100);

    // Moved with an arrow key (ten points to the right with Shift; the next press waits for the
    // document to take this one) ...
    await stamps(page).first().focus();
    await page.keyboard.press("Shift+ArrowRight");
    await expect.poll(async () => ((await placeOf(stamps(page)))?.x ?? stamp.x) - stamp.x).toBeGreaterThan(5);
    // ... and dragged, the dashed outline following the pointer until it is let go.
    await stamps(page).first().focus();
    await expect(page.locator("[data-move-resize]")).toBeVisible();
    const grip = await boxOf(page.locator("[data-move-resize]"));
    const gripX = grip.x + grip.width / 2;
    const gripY = grip.y + grip.height / 2;
    const beforeDrag = await boxOf(stamps(page));
    await page.mouse.move(gripX, gripY);
    await page.mouse.down();
    await page.mouse.move(gripX - 60, gripY + 40, { steps: 6 });
    await expect(page.locator("[data-move-preview]")).toBeVisible();
    await page.mouse.up();
    await expect
      .poll(async () => {
        const now = (await placeOf(stamps(page))) ?? beforeDrag;
        return [Math.round(now.x - beforeDrag.x), Math.round(now.y - beforeDrag.y)];
      })
      .toEqual([-60, 40]);
    const moved = await boxOf(stamps(page));
    await expect.poll(() => pixelsOf(page, moved, "red")).toBeGreaterThan(100);

    const copy = path.join(folder, "drawn.pdf");
    await page.keyboard.press("Control+Shift+S");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await quit(page);

    // Standard annotations, with where they are and how they look, and nothing of who made them.
    const saved = readFileSync(copy).toString("latin1");
    expect(saved).toMatch(/\/Subtype\s*\/Ink\b/);
    expect(saved).toMatch(/\/InkList/);
    expect(saved).toMatch(/\/Subtype\s*\/Stamp\b/);
    expect(saved).toMatch(/\/Name\s*\/Draft\b/);
    expect(saved).not.toMatch(/\/T\s*[(<]/);
    expect(saved).not.toMatch(/\/M\s*[(<]/);
    expect(saved).not.toMatch(/\/(NM|CreationDate)\b/);

    // The copy opens again with the line (only one: the second was undone) and the stamp.
    const reopened = await launch(copy);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    await expect(inks(reopened)).toHaveCount(1);
    await expect(stamps(reopened)).toHaveCount(1);
    await expect.poll(async () => pixelsOf(reopened, await boxOf(inks(reopened).first()), "blue")).toBeGreaterThan(50);
    await expect.poll(async () => pixelsOf(reopened, await boxOf(stamps(reopened).first()), "red")).toBeGreaterThan(100);
    // Taking the line away with Delete leaves the stamp.
    await inks(reopened).first().focus();
    await reopened.keyboard.press("Delete");
    await expect(inks(reopened)).toHaveCount(0);
    await expect(stamps(reopened)).toHaveCount(1);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("a stamp is put in the middle of the page with Enter, for those who cannot point, and Esc gives it up (B2-08)", async ({
  launch,
}) => {
  const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
  await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
  const sheet = await boxOf(firstPage(page));

  // Chosen and given up: nothing is put down, the page is as it was.
  await page.getByRole("button", { name: t.stamp }).click();
  await page.getByRole("menuitem", { name: t.stamps.final }).click();
  await expect(drawing(page)).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(drawing(page)).toHaveCount(0);
  await expect(stamps(page)).toHaveCount(0);

  // Chosen again, and Enter puts it in the middle of the page being read.
  await page.getByRole("button", { name: t.stamp }).click();
  await page.getByRole("menuitem", { name: t.stamps.final }).click();
  // Enter is the stamp's once the menu has closed (while it closes it is still the menu's).
  await expect(page.getByRole("menu")).toHaveCount(0);
  await page.keyboard.press("Enter");
  await expect(stamps(page)).toHaveCount(1);
  await expect(drawing(page)).toHaveCount(0);
  const stamp = await boxOf(stamps(page).first());
  expect(Math.abs(stamp.x + stamp.width / 2 - (sheet.x + sheet.width / 2))).toBeLessThan(3);
  expect(Math.abs(stamp.y + stamp.height / 2 - (sheet.y + sheet.height / 2))).toBeLessThan(3);
  await expect.poll(() => pixelsOf(page, stamp, "blue")).toBeGreaterThan(100);
});

test("the pen draws over a link instead of following it (B2-08)", async ({ launch }) => {
  const page = await launch(corpus("benign/internal-links.pdf"));
  await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
  const pageNumber = page.getByRole("textbox", { name: strings.toolbar.pageNumber });
  const link = page.locator('[data-page-links="1"] [data-link]').first();
  await expect(link).toBeVisible();
  const area = await boxOf(link);

  // With the pen on, a line across the link is a line, and the page stays where it is.
  await page.getByRole("button", { name: t.pen, exact: true }).click();
  const middle = area.y + area.height / 2;
  await draw(page, [
    [area.x + area.width * 0.2, middle],
    [area.x + area.width * 0.8, middle],
  ]);
  await expect(inks(page)).toHaveCount(1);
  await expect(pageNumber).toHaveValue("1");

  // With the pen put away, the same press follows the link (the line is no hindrance).
  await page.keyboard.press("Escape");
  await expect(drawing(page)).toHaveCount(0);
  await page.mouse.click(area.x + area.width * 0.5, middle);
  await expect(pageNumber).toHaveValue("3");
});
