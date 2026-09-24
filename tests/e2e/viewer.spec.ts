// Viewer layout in the real WebView (scroll bars do not exist in the unit tests' jsdom).
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

/** Letter pages are 792 x 612 pt; fit width adds 16 px of padding on each side (layout.ts). */
const PAGE_RATIO = 792 / 612;
const PADDING = 32;
const SCROLL_BAR = 15;

test("fit width stays put when the vertical scroll bar would come and go (#46)", async ({ launch }) => {
  const page = await launch(corpus("benign/single-page.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toBeVisible();
  const canvas = page.getByRole("main", { name: strings.canvas.label });

  // Make the canvas exactly as tall as the band where a fitted page is taller than the canvas
  // at full width, but shorter once a scroll bar has taken its width: there, a scroll bar that
  // takes space makes the layout flip between the two forever.
  const size = () =>
    canvas.evaluate((element) => ({ outer: element.getBoundingClientRect().width, height: element.clientHeight }));
  const viewport = page.viewportSize() ?? (await page.evaluate(() => ({ width: innerWidth, height: innerHeight })));
  const { outer, height } = await size();
  const tallest = (outer - PADDING) * PAGE_RATIO + PADDING;
  const shortest = (outer - SCROLL_BAR - PADDING) * PAGE_RATIO + PADDING;
  const target = Math.round((tallest + shortest) / 2);
  await page.setViewportSize({ width: viewport.width, height: viewport.height + (target - height) });
  await expect.poll(async () => (await size()).height, { timeout: 5_000 }).toBeGreaterThan(shortest);

  // Sample the layout for a while: it must settle on one width.
  const widths = new Set<string>();
  for (let sample = 0; sample < 15; sample++) {
    await page.waitForTimeout(100);
    widths.add(await page.getByRole("img", { name: strings.canvas.page(1) }).evaluate((element) => element.style.width));
  }
  expect([...widths]).toHaveLength(1);
});
