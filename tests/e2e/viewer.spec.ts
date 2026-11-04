// Viewer layout in the real WebView (scroll bars do not exist in the unit tests' jsdom).
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";
import { findOnScreen } from "./search";

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

test("fitted pages leave no horizontal scroll bar at 150% display scaling (#83)", async ({ launch }) => {
  // At 150% the vertical scroll bar is 23 device pixels, 15⅓ CSS pixels: the canvas is a third
  // of a pixel narrower than its clientWidth, which is rounded.
  const page = await launch(corpus("benign/single-page.pdf"), { deviceScaleFactor: 1.5 });
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  expect(await page.evaluate(() => devicePixelRatio)).toBe(1.5);
  const canvas = page.getByRole("main", { name: strings.canvas.label });
  // A horizontal scroll bar takes its height from the canvas.
  const scrollBarHeight = () => canvas.evaluate((element) => (element as HTMLElement).offsetHeight - element.clientHeight);

  for (const [key, zoom] of [
    ["Control+2", strings.toolbar.fitWidth],
    ["Control+0", strings.toolbar.fitPage],
  ] as const) {
    await page.keyboard.press(key);
    await expect(page.getByRole("contentinfo")).toContainText(zoom);
    await expect.poll(scrollBarHeight).toBe(0);
    await canvas.evaluate((element) => element.scrollTo({ left: 10 }));
    expect(await canvas.evaluate((element) => element.scrollLeft)).toBe(0);
  }

  // Wider than the canvas, the page does scroll sideways.
  await page.getByRole("combobox", { name: strings.toolbar.zoomLevel }).selectOption("400");
  await expect.poll(scrollBarHeight).toBeGreaterThan(0);
});

test("the search bar stays below the security banner and does not cover its buttons (#281)", async ({ launch }) => {
  const page = await launch(corpus("malicious/openaction-js.pdf"));
  const banner = page.getByRole("region", { name: strings.banner.label });
  await expect(banner).toBeVisible();

  // Open the search bar with Ctrl+F
  await page.keyboard.press("Control+f");
  const searchBar = page.getByRole("search");
  await expect(searchBar).toBeVisible();

  // The search bar is below the banner: its bounding box top is >= banner's bottom
  const bannerBox = await banner.boundingBox();
  const searchBox = await searchBar.boundingBox();
  expect(bannerBox).not.toBeNull();
  expect(searchBox).not.toBeNull();
  if (bannerBox && searchBox) {
    expect(searchBox.y).toBeGreaterThanOrEqual(bannerBox.y + bannerBox.height);
  }

  // The banner's buttons are clickable without the search bar intercepting pointer events
  const detailsButton = banner.getByRole("button", { name: strings.banner.details });
  await detailsButton.click();
  const detailsPanel = page.getByRole("complementary", { name: strings.banner.detailsTitle });
  await expect(detailsPanel).toBeVisible();

  // Closing the details panel also works
  const closeButton = detailsPanel.getByRole("button", { name: strings.banner.detailsClose });
  await closeButton.click();
  await expect(detailsPanel).toBeHidden();
});

test("navigation history: jumps and returns with Alt+Left, Alt+Right, and toolbar buttons (B4-01)", async ({ launch }) => {
  const page = await launch(corpus("benign/internal-links.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  const pageInput = page.getByRole("textbox", { name: strings.toolbar.pageNumber });
  const backButton = page.getByRole("button", { name: strings.toolbar.historyBack });
  const forwardButton = page.getByRole("button", { name: strings.toolbar.historyForward });

  // Initially at page 1; both back and forward are disabled
  await expect(pageInput).toHaveValue("1");
  await expect(backButton).toBeDisabled();
  await expect(forwardButton).toBeDisabled();

  // Track unload to ensure WebView2 does not reload or navigate away
  await page.evaluate(() => {
    (window as unknown as { __unloaded?: boolean }).__unloaded = false;
    window.addEventListener("beforeunload", () => {
      (window as unknown as { __unloaded?: boolean }).__unloaded = true;
    });
  });

  // Click the internal link on page 1 (which jumps to page 3)
  const link = page.locator('[data-page-links="1"] [data-link]').first();
  await expect(link).toBeVisible();
  await link.click();

  // Page is now 3; back button is enabled, forward is disabled
  await expect(pageInput).toHaveValue("3");
  await expect(backButton).toBeEnabled();
  await expect(forwardButton).toBeDisabled();

  // Press Alt+Left to go back to page 1
  await page.keyboard.press("Alt+ArrowLeft");
  await expect(pageInput).toHaveValue("1");
  await expect(backButton).toBeDisabled();
  await expect(forwardButton).toBeEnabled();

  // Verify page did not reload
  const unloaded = await page.evaluate(() => (window as unknown as { __unloaded?: boolean }).__unloaded);
  expect(unloaded).toBe(false);

  // Press Alt+Right to go forward to page 3
  await page.keyboard.press("Alt+ArrowRight");
  await expect(pageInput).toHaveValue("3");
  await expect(backButton).toBeEnabled();
  await expect(forwardButton).toBeDisabled();

  // Click the back button directly
  await backButton.click();
  await expect(pageInput).toHaveValue("1");
  await expect(backButton).toBeDisabled();
  await expect(forwardButton).toBeEnabled();

  // Click the forward button directly
  await forwardButton.click();
  await expect(pageInput).toHaveValue("3");
  await expect(backButton).toBeEnabled();
  await expect(forwardButton).toBeDisabled();
});

test("two-page view with cover page: jumps to page 5, highlights text on page 5, and searches for page 7 (B4-02)", async ({
  launch,
}) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // 1. Switch to "雙頁（封面單獨）", input page 5
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitemradio", { name: strings.menu.layoutTwoPageCover }).click();

  const pageInput = page.getByRole("textbox", { name: strings.toolbar.pageNumber });
  await pageInput.fill("5");
  await pageInput.press("Enter");

  const page4 = page.getByRole("img", { name: strings.canvas.page(4) });
  const page5 = page.getByRole("img", { name: strings.canvas.page(5) });
  await expect(page4).toBeVisible();
  await expect(page5).toBeVisible();

  // 第 4、5 頁的上緣切齊，第 4 頁在左；頁碼欄位是 5
  const box4 = await page4.boundingBox();
  const box5 = await page5.boundingBox();
  expect(box4).not.toBeNull();
  expect(box5).not.toBeNull();
  expect(Math.abs(box4!.y - box5!.y)).toBeLessThanOrEqual(1);
  expect(box4!.x + box4!.width).toBeLessThan(box5!.x);
  await expect(pageInput).toHaveValue("5");

  // 2. 在第 5 頁選取文字、加螢光筆（照 annotations.spec.ts 的寫法）：螢光筆在第 5 頁
  // Multi-page 10 has "Page 5 of 10" on page 5
  const textHit = await findOnScreen(page, "Page 5");
  await page.keyboard.press("Escape");
  await page.mouse.dblclick(textHit.x + textHit.width / 4, textHit.y + textHit.height / 2);
  await page.mouse.click(textHit.x + textHit.width / 4, textHit.y + textHit.height / 2, { button: "right" });
  await page.getByRole("menuitem", { name: strings.annotations.highlight }).click();
  await page.getByRole("menuitem", { name: strings.annotations.colors.yellow }).click();

  const mark = page.locator("[data-page-annotations] [data-annotation=highlight]");
  await expect(mark).toHaveCount(1);
  const markBox = await mark.first().boundingBox();
  expect(markBox).not.toBeNull();
  // 螢光筆落在第 5 頁內
  expect(markBox!.x).toBeGreaterThanOrEqual(box5!.x - 1);
  expect(markBox!.x + markBox!.width).toBeLessThanOrEqual(box5!.x + box5!.width + 1);

  // 3. 搜尋第 7 頁上的關鍵字：第 7 頁出現在畫面上
  // Keyword "needle" appears only on page 7
  await findOnScreen(page, "needle");
  const page7 = page.getByRole("img", { name: strings.canvas.page(7) });
  await expect(page7).toBeInViewport();
});

