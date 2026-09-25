// Selecting and copying text (MVP-15) in the real app. Where a word is on screen comes from
// search, whose highlights are checked against the text by MVP-10's tests: so these tests need
// no page coordinates, and hold at any zoom and rotation.
import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

const CHINESE = "隱私優先的 PDF 閱讀器";

async function open(launch: (file?: string) => Promise<Page>) {
  const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  // Copied text is recorded instead of put on this computer's clipboard.
  await page.evaluate(() => {
    const copied: string[] = [];
    Object.assign(window, { copied });
    navigator.clipboard.writeText = async (text: string) => {
      copied.push(text);
    };
  });
  const copied = () => page.evaluate(() => (window as unknown as { copied: string[] }).copied.at(-1));
  return { page, copied };
}

/** Searches for `query` and returns where its only hit is on screen. */
async function find(page: Page, query: string) {
  await page.keyboard.press("Control+f");
  const input = page.getByRole("textbox", { name: strings.search.placeholder });
  await input.fill(query);
  await input.press("Enter");
  await expect(page.locator("[data-highlights] polygon[data-current]")).toHaveCount(1);
  // The hit's fill: the outline of the current hit is drawn with a stroke, which widens its box.
  const hit = page.locator("[data-highlights] polygon:not([data-current])");
  await expect(hit).toHaveCount(1);
  const box = await hit.boundingBox();
  if (!box) throw new Error(`${query} is not on screen`);
  return box;
}

test("a double click selects a word and Ctrl+C copies it, in English and in Chinese", async ({ launch }) => {
  const { page, copied } = await open(launch);

  const privacy = await find(page, "Privacy");
  await page.mouse.dblclick(privacy.x + privacy.width / 2, privacy.y + privacy.height / 2);
  await page.keyboard.press("Control+c");
  await expect.poll(copied).toBe("Privacy");

  const reader = await find(page, "閱讀器");
  await page.mouse.dblclick(reader.x + reader.width / 2, reader.y + reader.height / 2);
  await page.keyboard.press("Control+c");
  // However the word breaker splits 閱讀器, the clicked 讀 is in the copied word.
  await expect.poll(copied).toContain("讀");
});

test("dragging selects across lines, and the context menu copies", async ({ launch }) => {
  const { page, copied } = await open(launch);
  // Near the top of the page, neither search scrolls the view.
  const end = await find(page, "閱讀器");
  const start = await find(page, "Privacy");

  await page.mouse.move(start.x + 1, start.y + start.height / 2);
  await page.mouse.down();
  await page.mouse.move(end.x + end.width - 1, end.y + end.height / 2, { steps: 8 });
  await page.mouse.up();

  await page.mouse.click(end.x, end.y + end.height / 2, { button: "right" });
  await page.getByRole("menuitem", { name: new RegExp(strings.text.copy) }).click();
  await expect.poll(copied).toBe(`Privacy-first PDF Reader\n${CHINESE}`);
});

test("the selection covers the text when the page is turned and zoomed to 400%", async ({ launch }) => {
  const { page, copied } = await open(launch);
  await page.keyboard.press("Control+]");
  await page.getByRole("combobox", { name: strings.toolbar.zoomLevel }).selectOption("400");

  const hit = await find(page, "Privacy");
  await page.mouse.dblclick(hit.x + hit.width / 2, hit.y + hit.height / 2);
  const selection = page.locator("[data-selection] polygon");
  await expect(selection).toHaveCount(1);
  const selected = await selection.boundingBox();
  // Turned a quarter, the word runs down the screen; the selection covers exactly what search found.
  expect(hit.height).toBeGreaterThan(hit.width);
  for (const side of ["x", "y", "width", "height"] as const) {
    expect(Math.abs(selected![side] - hit[side])).toBeLessThan(2);
  }
  await page.keyboard.press("Control+c");
  await expect.poll(copied).toBe("Privacy");
});
