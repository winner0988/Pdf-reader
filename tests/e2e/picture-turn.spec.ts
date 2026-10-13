// Turning a picture stamp (B3-01) in the real app. The handle above a chosen picture turns it, the
// buttons in its toolbar and the keys [ and ] turn it a step; it stays turned when it is moved and
// resized, in the copy that is saved, and when that copy is opened again. A standard stamp has
// none of this.
import { copyFileSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Locator, Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const t = strings.annotations;

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) });
const stamps = (page: Page) => page.locator("[data-page-annotations] [data-annotation=stamp]");
/** The handle above the chosen picture, which is dragged to turn it. */
const handle = (page: Page) => page.locator("[data-turn-handle]");

type Box = { x: number; y: number; width: number; height: number };

async function boxOf(locator: Locator): Promise<Box> {
  const box = await locator.boundingBox();
  if (!box) throw new Error("it is not on screen");
  return box;
}

/** Where the first of `locator` is, or `null` while it is not (an edit gives the page new annotations). */
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

const OPPOSITE = { top: "bottom", right: "left", bottom: "top", left: "right" } as const;
type Side = keyof typeof OPPOSITE;

/** The pixels either side of the line down the middle of the picture, where its two colours blend. */
const GUTTER_PX = 4;

/** The half of `box` on `side`, but for the gutter. */
function half(box: Box, side: Side): Box {
  const [width, height] = [box.width / 2 - GUTTER_PX, box.height / 2 - GUTTER_PX];
  switch (side) {
    case "top":
      return { ...box, height };
    case "bottom":
      return { ...box, y: box.y + box.height / 2 + GUTTER_PX, height };
    case "left":
      return { ...box, width };
    case "right":
      return { ...box, x: box.x + box.width / 2 + GUTTER_PX, width };
  }
}

/**
 * The page shows the picture (red over blue, upright) with its red half on `side` of `box`, its
 * blue half on the opposite side, and no red there.
 */
async function expectRedOn(page: Page, box: Box, side: Side) {
  const other = OPPOSITE[side];
  await expect.poll(() => pixelsOf(page, half(box, side), "red")).toBeGreaterThan(100);
  await expect.poll(() => pixelsOf(page, half(box, other), "blue")).toBeGreaterThan(100);
  expect(await pixelsOf(page, half(box, other), "red")).toBeLessThan(30);
  expect(await pixelsOf(page, half(box, side), "blue")).toBeLessThan(30);
}

/** How far the chosen picture's outline is turned clockwise, in degrees, read off its first two corners. */
function turnOf(page: Page): Promise<number> {
  return page.locator("[data-turn-outline]").evaluate((outline) => {
    const [a, b] = (outline.getAttribute("points") ?? "")
      .split(" ")
      .map((pair) => pair.split(",").map(Number));
    return Math.round((Math.atan2(b![1]! - a![1]!, b![0]! - a![0]!) * 1800) / Math.PI) / 10;
  });
}

/** Chooses the first stamp, as the keyboard does. */
async function choose(page: Page) {
  await stamps(page).first().focus();
  await expect(page.locator("[data-move-resize]")).toBeVisible();
}

/** Fits the whole page on the screen: its text is at the top, the rest is white, where stamps go. */
async function fitPage(page: Page): Promise<Box> {
  await page.keyboard.press("Control+0");
  await expect.poll(async () => (await boxOf(firstPage(page))).height).toBeLessThan(700);
  return boxOf(firstPage(page));
}

/** Drags the chosen picture's handle to `(x, y)` on the screen and lets go. */
async function dragHandle(page: Page, to: [number, number]) {
  const grip = await boxOf(handle(page));
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(...to, { steps: 8 });
}

test("a picture is turned with the handle, the buttons and the keys, stays turned when it is moved and resized, and is turned in the copy (B3-01)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-turn-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    const sheet = await fitPage(page);

    // A picture of the user's own, put down in the white of the page: red over blue, two to one.
    await page.getByRole("button", { name: t.stamp }).click();
    await page.getByRole("menuitem", { name: t.pickPicture }).click();
    await answerFileDialog(page, { path: corpus("images/stamp-exif.jpg") });
    await page.mouse.click(sheet.x + sheet.width * 0.5, sheet.y + sheet.height * 0.6);
    await expect(stamps(page)).toHaveCount(1);
    const upright = await boxOf(stamps(page));
    expect(Math.abs(upright.width / upright.height - 2)).toBeLessThan(0.2);
    await expectRedOn(page, upright, "top");

    // The handle is above it. Dragged to the right of its middle the picture is turned a quarter:
    // the outline follows the pointer, and the page changes when the button is let go.
    await choose(page);
    const middle = { x: upright.x + upright.width / 2, y: upright.y + upright.height / 2 };
    const grip = await boxOf(handle(page));
    const reach = middle.y - (grip.y + grip.height / 2);
    expect(reach).toBeGreaterThan(upright.height / 2);
    await dragHandle(page, [middle.x + reach, middle.y]);
    await expect.poll(() => turnOf(page)).toBe(90);
    expect(await pixelsOf(page, half(upright, "bottom"), "red")).toBeLessThan(30);
    await page.mouse.up();
    await expect.poll(async () => ((await placeOf(stamps(page))) ?? upright).height).toBeGreaterThan(upright.height * 1.5);
    const quarter = await boxOf(stamps(page));
    expect(Math.abs(quarter.width - upright.height)).toBeLessThan(3);
    expect(Math.abs(quarter.height - upright.width)).toBeLessThan(3);
    expect(Math.abs(quarter.x + quarter.width / 2 - middle.x)).toBeLessThan(3);
    expect(Math.abs(quarter.y + quarter.height / 2 - middle.y)).toBeLessThan(3);
    await expectRedOn(page, quarter, "right");
    expect(await turnOf(page)).toBe(90);

    // A step to the right with the button, and to the left with the key (each is an edit, which the
    // next waits for).
    await choose(page);
    await page.getByRole("button", { name: t.turnRight }).click();
    await expect.poll(() => turnOf(page)).toBe(105);
    await choose(page);
    await page.keyboard.press("[");
    await expect.poll(() => turnOf(page)).toBe(90);
    await choose(page);
    await page.keyboard.press("[");
    await expect.poll(() => turnOf(page)).toBe(75);
    await page.getByRole("button", { name: t.turnRight }).click();
    await expect.poll(() => turnOf(page)).toBe(90);
    await expectRedOn(page, await boxOf(stamps(page)), "right");

    // Moved with an arrow key, it is where it was sent and still turned: the engine would draw a
    // picture upright again if it were asked to redraw it.
    const before = await boxOf(stamps(page));
    await choose(page);
    await page.keyboard.press("Shift+ArrowRight");
    await expect.poll(async () => ((await placeOf(stamps(page))) ?? before).x - before.x).toBeGreaterThan(5);
    const moved = await boxOf(stamps(page));
    expect(Math.abs(moved.width - before.width)).toBeLessThan(2);
    expect(Math.abs(moved.height - before.height)).toBeLessThan(2);
    expect(await turnOf(page)).toBe(90);
    await expectRedOn(page, moved, "right");

    // Resized by a corner, it keeps its shape and its turn.
    await choose(page);
    const corner = await boxOf(page.locator("[data-move-resize] [data-handle=se]"));
    const [cornerX, cornerY] = [corner.x + corner.width / 2, corner.y + corner.height / 2];
    await page.mouse.move(cornerX, cornerY);
    await page.mouse.down();
    await page.mouse.move(cornerX + 40, cornerY + 80, { steps: 6 });
    await page.mouse.up();
    await expect.poll(async () => ((await placeOf(stamps(page))) ?? moved).height).toBeGreaterThan(moved.height + 20);
    const grown = await boxOf(stamps(page));
    expect(Math.abs(grown.width / grown.height - moved.width / moved.height)).toBeLessThan(0.05);
    expect(await turnOf(page)).toBe(90);
    await expectRedOn(page, grown, "right");

    // Undone and made again: the smaller picture is as turned as the larger.
    await page.keyboard.press("Control+z");
    await expect.poll(async () => ((await placeOf(stamps(page))) ?? grown).height).toBeLessThan(grown.height - 10);
    await expectRedOn(page, await boxOf(stamps(page)), "right");
    await page.keyboard.press("Control+y");
    await expect.poll(async () => ((await placeOf(stamps(page))) ?? moved).height).toBeGreaterThan(moved.height + 20);
    await expectRedOn(page, await boxOf(stamps(page)), "right");

    // The copy has a standard stamp whose appearance is turned a quarter, and opens turned.
    const copy = path.join(folder, "turned.pdf");
    await page.keyboard.press("Control+Shift+S");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await quit(page);

    const saved = readFileSync(copy).toString("latin1");
    expect(saved).toMatch(/\/Subtype\s*\/Stamp/);
    const matrices = [...saved.matchAll(/\/Matrix\s*\[([^\]]*)\]/g)].map((found) =>
      found[1]!.trim().split(/\s+/).map(Number),
    );
    const aQuarterClockwise = matrices.some(
      ([a, b, c, d]) =>
        Math.abs(a!) < 0.01 && Math.abs(b! + 1) < 0.01 && Math.abs(c! - 1) < 0.01 && Math.abs(d!) < 0.01,
    );
    expect(aQuarterClockwise).toBe(true);

    const reopened = await launch(copy);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    await fitPage(reopened);
    await expect(stamps(reopened)).toHaveCount(1);
    const there = await boxOf(stamps(reopened));
    expect(there.height).toBeGreaterThan(there.width * 1.5);
    await expectRedOn(reopened, there, "right");

    // A picture that was opened turned is turned back up with the handle: the handle is to the
    // right of it now, and dragged above its middle the picture is upright again.
    await choose(reopened);
    const knob = await boxOf(handle(reopened));
    const centre = { x: there.x + there.width / 2, y: there.y + there.height / 2 };
    const distance = knob.x + knob.width / 2 - centre.x;
    expect(distance).toBeGreaterThan(there.width / 2);
    await dragHandle(reopened, [centre.x, centre.y - distance]);
    await reopened.mouse.up();
    await expect.poll(async () => ((await placeOf(stamps(reopened))) ?? there).width).toBeGreaterThan(there.width * 1.5);
    // Upright it is as large as it was when it was turned: the size it was resized to is kept.
    const upAgain = await boxOf(stamps(reopened));
    expect(Math.abs(upAgain.width - there.height)).toBeLessThan(3);
    expect(Math.abs(upAgain.height - there.width)).toBeLessThan(3);
    await expectRedOn(reopened, upAgain, "top");
    expect(await turnOf(reopened)).toBe(0);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("a standard stamp has no handle and no buttons to turn it, and the keys do nothing to it (B3-01)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-turn-standard-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    const sheet = await fitPage(page);

    await page.getByRole("button", { name: t.stamp }).click();
    await page.getByRole("menuitem", { name: t.stamps.draft }).click();
    await page.mouse.click(sheet.x + sheet.width * 0.5, sheet.y + sheet.height * 0.6);
    await expect(stamps(page)).toHaveCount(1);
    const placed = await boxOf(stamps(page));

    // It is chosen and can be moved and resized, but not turned.
    await choose(page);
    await expect(page.getByRole("button", { name: t.delete })).toBeVisible();
    await expect(handle(page)).toHaveCount(0);
    await expect(page.getByRole("button", { name: t.turnRight })).toHaveCount(0);
    await expect(page.getByRole("button", { name: t.turnLeft })).toHaveCount(0);
    await page.keyboard.press("]");
    await page.keyboard.press("[");
    await page.waitForTimeout(500);
    const still = await boxOf(stamps(page));
    expect(Math.abs(still.x - placed.x) + Math.abs(still.y - placed.y)).toBeLessThan(1);
    expect(Math.abs(still.width - placed.width) + Math.abs(still.height - placed.height)).toBeLessThan(1);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
