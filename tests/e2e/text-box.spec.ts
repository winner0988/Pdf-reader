// The text boxes (B2-30, B2-31, #719) in the real app, with the fonts this Windows really has:
// typed directly on the page with a floating toolbar (no dialog), auto-growing without clipping,
// matching line breaks before and after commit, found by search and copied, saved, and shown again
// when the copy is opened; moved, resized, re-worded in one undoable edit, deleted and brought
// back; a crash leaves it to restore; and the fonts that cannot be used say why.
//
// Which fonts a computer has is its own: each script names the families it is happy with, and the
// test takes the first one the toolbar offers. A script none of whose families is installed is a
// failure that lists what the toolbar offers, not a test that quietly does less.
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Locator, Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, dataDir, expect, quit, test } from "./app";

const t = strings.annotations;
const e = t.textBoxEditor;

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) });
const boxes = (page: Page) => page.locator("[data-page-annotations] [data-annotation=textBox]");
/** The layer a tool is used on: there while the text box tool is on. */
const drawing = (page: Page) => page.locator("[data-page-drawing]");
/** The active inline text box `<textarea>`. */
const inlineInput = (page: Page) => page.getByRole("textbox", { name: e.label });
/** The floating formatting toolbar shown while typing inline. */
const editorToolbar = (page: Page) => page.getByRole("toolbar", { name: e.toolbarLabel });

type Box = { x: number; y: number; width: number; height: number };

async function boxOf(locator: Locator): Promise<Box> {
  const box = await locator.boundingBox();
  if (!box) throw new Error("it is not on screen");
  return box;
}

/**
 * How much blue ink (the toolbar's blue: 26, 77, 217) the first page, as drawn on its canvas, has
 * inside `box` of the screen. Counts pixels that are clearly bluer than they are red, so the
 * thin strokes of small text, which are never a whole pixel of the colour, count too.
 */
async function inkOf(page: Page, box: Box): Promise<number> {
  return firstPage(page)
    .locator("canvas:not([data-detail])")
    .evaluate((canvas: HTMLCanvasElement, area) => {
      const bounds = canvas.getBoundingClientRect();
      const scaleX = canvas.width / bounds.width;
      const scaleY = canvas.height / bounds.height;
      const x0 = Math.max(0, Math.floor((area.x - bounds.left) * scaleX));
      const y0 = Math.max(0, Math.floor((area.y - bounds.top) * scaleY));
      const x1 = Math.min(canvas.width, Math.ceil((area.x + area.width - bounds.left) * scaleX));
      const y1 = Math.min(canvas.height, Math.ceil((area.y + area.height - bounds.top) * scaleY));
      const data = canvas.getContext("2d")!.getImageData(x0, y0, Math.max(x1 - x0, 1), Math.max(y1 - y0, 1)).data;
      let count = 0;
      for (let at = 0; at < data.length; at += 4) {
        if (data[at + 2]! - data[at]! > 40 && data[at + 2]! > 120) count++;
      }
      return count;
    }, box);
}

/** Enough blue pixels to be words in a 24 point font, and not a stray antialiased edge. */
const SHOWS_WORDS = 100;

type Script = { name: string; families: string[]; words: string; query: string };

const SCRIPTS: Script[] = [
  { name: "English", families: ["Arial", "Segoe UI", "Verdana"], words: "Zebra crossing", query: "Zebra" },
  {
    name: "Traditional Chinese",
    families: ["Microsoft JhengHei", "PMingLiU", "MingLiU", "DFKai-SB"],
    words: "方塊測試文字",
    query: "測試",
  },
  {
    name: "Simplified Chinese",
    families: ["Microsoft YaHei", "SimSun", "SimHei", "DengXian"],
    words: "简体字测试",
    query: "测试",
  },
  {
    name: "Japanese",
    families: ["Yu Gothic", "Meiryo", "MS Gothic", "MS PGothic", "Yu Mincho"],
    words: "日本語のテスト",
    query: "テスト",
  },
];

/** The first of `candidates` the font menu of `toolbar` offers; fails with what it does offer. */
async function offered(toolbar: Locator, candidates: string[]): Promise<string> {
  const menu = toolbar.getByLabel(e.font, { exact: true });
  const families = await menu.locator("option").allTextContents();
  const found = candidates.find((family) => families.includes(family));
  if (found === undefined) {
    throw new Error(`none of ${candidates.join(", ")} is installed; the toolbar offers: ${families.join(", ")}`);
  }
  return found;
}

type Filling = { words: string; families: string[]; size?: number; blue?: boolean };

/** Fills in the open inline editor + floating toolbar. */
async function fillIn(page: Page, filling: Filling): Promise<string> {
  const bar = editorToolbar(page);
  const family = await offered(bar, filling.families);
  await inlineInput(page).fill(filling.words);
  await bar.getByLabel(e.font, { exact: true }).selectOption({ label: family });
  await bar.getByLabel(e.size, { exact: true }).fill(String(filling.size ?? 24));
  if (filling.blue !== false) await bar.getByRole("radio", { name: e.colors.blue }).click();
  return family;
}

/** Whether the page is ready and the whole sheet is on the screen (its upper part has text; the rest is white). */
async function ready(page: Page) {
  await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
  await page.keyboard.press("Control+0");
  await expect.poll(async () => (await firstPage(page).boundingBox())?.height ?? Infinity).toBeLessThan(700);
}

/** Where, as a share of the first page's width and height, on the screen. */
async function at(page: Page, across: number, down: number): Promise<[number, number]> {
  const sheet = await boxOf(firstPage(page));
  return [sheet.x + sheet.width * across, sheet.y + sheet.height * down];
}

/**
 * Puts a text box down the way a user does: the tool's button, a click where it is to be, typing
 * directly on the page, and finishing by pressing Escape (or clicking outside). Returns the family.
 */
async function putDown(page: Page, where: [number, number], filling: Filling): Promise<string> {
  await page.getByRole("button", { name: t.textBox, exact: true }).click();
  await expect(drawing(page)).toHaveCount(1);
  await page.mouse.click(...where);
  await expect(inlineInput(page)).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const family = await fillIn(page, filling);
  // Click outside on empty top-left corner of the page to finish typing.
  const [outX, outY] = await at(page, 0.05, 0.95);
  await page.mouse.click(outX, outY);
  await expect(inlineInput(page)).toHaveCount(0);
  // The tool puts itself away once it has put a box down.
  await expect(drawing(page)).toHaveCount(0);
  return family;
}

/** Searches the open document for `query`, whose only hit it expects; returns where the hit is on screen. */
async function find(page: Page, query: string): Promise<Box> {
  await page.keyboard.press("Control+f");
  const search = page.getByRole("search", { name: strings.search.label });
  const input = search.getByRole("textbox", { name: strings.search.placeholder });
  await input.fill(query);
  await expect(search.getByRole("status")).toHaveText(strings.search.count(1, 1));
  await input.press("Enter");
  await expect(page.locator("[data-highlights] polygon[data-current]")).toHaveCount(1);
  // The hit's fill: the outline of the current hit is drawn with a stroke, which widens its box.
  const hit = page.locator("[data-highlights] polygon:not([data-current])");
  await expect(hit).toHaveCount(1);
  return boxOf(hit);
}

/** Records what the page copies, instead of putting it on this computer's clipboard (text.spec.ts). */
async function recordCopies(page: Page) {
  await page.evaluate(() => {
    const copied: string[] = [];
    Object.assign(window, { copied });
    navigator.clipboard.writeText = async (text: string) => {
      copied.push(text);
    };
  });
  return () => page.evaluate(() => (window as unknown as { copied: string[] }).copied.at(-1));
}

/** Reads the visual lines wrapped inside the active inline text box mirror before commit. */
async function visualLinesWhileTyping(page: Page): Promise<string[]> {
  return page.locator('[data-text-box-mirror="height"]').evaluate((el) => {
    const node = el.firstChild;
    if (!node || node.nodeType !== Node.TEXT_NODE) return [];
    const raw = (node.textContent ?? "").replace(/\u200b/g, "");
    const lines: { top: number; text: string }[] = [];
    const range = document.createRange();
    for (let i = 0; i < raw.length; i++) {
      const ch = raw[i]!;
      if (ch === "\n") continue;
      range.setStart(node, i);
      range.setEnd(node, i + 1);
      const rect = range.getBoundingClientRect();
      if (!(rect.height > 0)) continue;
      const existing = lines.find((line) => Math.abs(line.top - rect.top) < rect.height * 0.5);
      if (existing) existing.text += ch;
      else lines.push({ top: rect.top, text: ch });
    }
    return lines.map((line) => line.text.trimEnd());
  });
}

/** Whether `inner` lies inside `outer`, give or take `slack` pixels. */
function inside(inner: Box, outer: Box, slack = 3): boolean {
  return (
    inner.x >= outer.x - slack &&
    inner.y >= outer.y - slack &&
    inner.x + inner.width <= outer.x + outer.width + slack &&
    inner.y + inner.height <= outer.y + outer.height + slack
  );
}

/** Saves the document as a copy at `copy` (the system's dialog), once it says it is saved. */
async function saveCopy(page: Page, copy: string) {
  await page.keyboard.press("Control+Shift+S");
  await answerFileDialog(page, { path: copy });
  await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
}

const folderFor = (name: string) => mkdtempSync(path.join(tmpdir(), `pdf-reader-e2e-${name}-`));
const removeFolder = (folder: string) =>
  rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });

for (const script of SCRIPTS) {
  test(`${script.name} words in an inline text box are shown, found by search, copied, saved, and shown again (B2-31, #719)`, async ({
    launch,
  }) => {
    test.setTimeout(120_000);
    const folder = folderFor("text-box");
    try {
      const file = path.join(folder, "original.pdf");
      copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
      const page = await launch(file);
      await ready(page);
      await expect(boxes(page)).toHaveCount(0);

      // Put down where the user clicked, in the first font of this script the computer has.
      const place = await at(page, 0.25, 0.55);
      const family = await putDown(page, place, { words: script.words, families: script.families });
      await expect(boxes(page)).toHaveCount(1);
      await expect(boxes(page).first()).toHaveAccessibleName(t.textBoxSaying(script.words));
      const placed = await boxOf(boxes(page).first());
      expect(Math.abs(placed.x - place[0])).toBeLessThan(6);
      // The page shows the words, in the chosen colour, inside the box.
      await expect.poll(() => inkOf(page, placed), { message: `${family} on the page` }).toBeGreaterThan(SHOWS_WORDS);

      // The words of a box are found by search, where the box is, before they are saved ...
      const hit = await find(page, script.query);
      expect(inside(hit, placed), `the hit ${JSON.stringify(hit)} is in the box ${JSON.stringify(placed)}`).toBe(true);
      // ... and are selected and copied like the page's own: by dragging across them.
      const copied = await recordCopies(page);
      await page.mouse.move(hit.x + 1, hit.y + hit.height / 2);
      await page.mouse.down();
      await page.mouse.move(hit.x + hit.width - 1, hit.y + hit.height / 2, { steps: 6 });
      await page.mouse.up();
      await page.keyboard.press("Control+c");
      await expect.poll(copied).toContain(script.query);

      await saveCopy(page, path.join(folder, "boxed.pdf"));
      await quit(page);

      // A standard annotation with the words, set in a subset of the font: the file has the few
      // glyphs it uses, not the font (the fonts of Windows are megabytes).
      const copy = path.join(folder, "boxed.pdf");
      const saved = readFileSync(copy).toString("latin1");
      expect(saved).toMatch(/\/Subtype\s*\/FreeText\b/);
      expect(saved).toMatch(/\/PdfReaderTextBox\b/);
      expect(statSync(copy).size - statSync(file).size, `${family} in the copy`).toBeLessThan(400_000);

      // The copy opens with the box as it was, and shows the words, and they are found again.
      const reopened = await launch(copy);
      await ready(reopened);
      await expect(boxes(reopened)).toHaveCount(1);
      await expect(boxes(reopened).first()).toHaveAccessibleName(t.textBoxSaying(script.words));
      await expect.poll(() => inkNow(reopened)).toBeGreaterThan(SHOWS_WORDS);
      const again = await find(reopened, script.query);
      expect(inside(again, await boxOf(boxes(reopened).first()))).toBe(true);
    } finally {
      removeFolder(folder);
    }
  });
}

/** Where the first of `locator` is, or `null` while it is not (an edit gives the page new annotations). */
const placeOf = (locator: Locator) => locator.first().boundingBox();

/** How much ink the first text box shows now; none while it is not there. */
async function inkNow(page: Page): Promise<number> {
  const box = await placeOf(boxes(page));
  return box ? inkOf(page, box) : 0;
}

const LATIN = ["Arial", "Segoe UI", "Verdana"];

test("an inline text box auto-grows without clipping, wraps consistently for English and Chinese, and edits undo in one step (#719)", async ({
  launch,
}) => {
  test.setTimeout(120_000);
  const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
  await ready(page);
  const copied = await recordCopies(page);

  const checkWrapping = async (
    where: [number, number],
    families: string[],
    text: string,
    label: string,
  ) => {
    await page.getByRole("button", { name: t.textBox, exact: true }).click();
    await page.mouse.click(...where);
    await expect(inlineInput(page)).toBeVisible();
    const bar = editorToolbar(page);
    const family = await offered(bar, families);
    await bar.getByLabel(e.font, { exact: true }).selectOption({ label: family });
    await bar.getByLabel(e.size, { exact: true }).fill("16");
    await bar.getByRole("radio", { name: e.colors.blue }).click();
    await inlineInput(page).fill(text);
    // Wait for background prepare_text_box (400ms debounce) so lineStep matches the font.
    await page.waitForTimeout(600);
    const linesBefore = await visualLinesWhileTyping(page);
    expect(linesBefore.length, `${label} should wrap onto multiple lines`).toBeGreaterThanOrEqual(2);

    // Finish by clicking empty corner.
    const [outX, outY] = await at(page, 0.05, 0.95);
    await page.mouse.click(outX, outY);
    await expect(inlineInput(page)).toHaveCount(0);

    const created = boxes(page).last();
    await expect(created).toHaveAccessibleName(t.textBoxSaying(text));
    const area = await boxOf(created);
    await expect.poll(() => inkOf(page, area)).toBeGreaterThan(SHOWS_WORDS);

    // Re-enter inline edit to verify auto-grow left zero overflow.
    await created.focus();
    await page.keyboard.press("Enter");
    await expect(inlineInput(page)).toBeVisible();
    await expect(editorToolbar(page).getByText(e.overflow)).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(inlineInput(page)).toHaveCount(0);

    // Drag across the entire box and copy to read the worker's actual line breaks.
    await page.mouse.move(area.x + 2, area.y + 4);
    await page.mouse.down();
    await page.mouse.move(area.x + area.width - 2, area.y + area.height - 4, { steps: 10 });
    await page.mouse.up();
    await page.keyboard.press("Control+c");
    await expect.poll(copied).toBeTruthy();
    const linesAfter = (await copied())!
      .split(/\r?\n/)
      .map((line) => line.trimEnd())
      .filter((line) => line.length > 0);
    expect(linesAfter, `${label} (${family}) line breaks before vs after commit`).toEqual(linesBefore);
  };

  // English paragraph wrapping near right half of page so it wraps onto 2+ lines.
  await checkWrapping(
    await at(page, 0.45, 0.4),
    LATIN,
    "Inline text boxes grow downward when long sentences reach the edge of the page cleanly.",
    "English",
  );

  // Traditional Chinese paragraph wrapping near right half of page.
  await checkWrapping(
    await at(page, 0.45, 0.65),
    SCRIPTS[1]!.families,
    "在頁面上直接輸入長段中文文字時，文字方塊會自動換行並往下延伸，不會截斷內容。",
    "Chinese",
  );

  // Single undo check: edit the second box to add more lines so both text and height change,
  // then press Ctrl+Z ONCE and verify both text and box height revert together.
  const second = boxes(page).nth(1);
  const beforeBox = await boxOf(second);
  await second.focus();
  await page.keyboard.press("Enter");
  await expect(inlineInput(page)).toBeVisible();
  await inlineInput(page).fill(
    "在頁面上直接輸入長段中文文字時，文字方塊會自動換行並往下延伸，不會截斷內容。\n第二段新增文字讓方塊再長高。\n第三段繼續增加高度。",
  );
  await page.keyboard.press("Escape");
  await expect(inlineInput(page)).toHaveCount(0);
  await expect(second).toHaveAccessibleName(/第三段繼續增加高度/);
  const grownBox = await boxOf(second);
  expect(grownBox.height).toBeGreaterThan(beforeBox.height + 10);

  // ONE Ctrl+Z restores BOTH original text and original height.
  await page.keyboard.press("Control+z");
  await expect(second).not.toHaveAccessibleName(/第三段繼續增加高度/);
  await expect
    .poll(async () => Math.abs(((await second.boundingBox())?.height ?? 0) - beforeBox.height))
    .toBeLessThan(3);

  // ONE Ctrl+Y restores BOTH updated text and grown height.
  await page.keyboard.press("Control+y");
  await expect(second).toHaveAccessibleName(/第三段繼續增加高度/);
  await expect
    .poll(async () => Math.abs(((await second.boundingBox())?.height ?? 0) - grownBox.height))
    .toBeLessThan(3);
});

test("a text box is moved, resized, re-worded, deleted and brought back, and the copy has it as it ended (B2-31, #719)", async ({
  launch,
}) => {
  test.setTimeout(120_000);
  const folder = folderFor("text-box-edit");
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const page = await launch(file);
    await ready(page);

    // The tool's own keyboard way: its button, then Enter starts inline typing in the middle of the page.
    await page.getByRole("button", { name: t.textBox, exact: true }).click();
    await expect(drawing(page)).toHaveCount(1);
    await page.keyboard.press("Enter");
    await expect(inlineInput(page)).toBeVisible();
    const family = await fillIn(page, { words: "Move me", families: LATIN });
    await page.keyboard.press("Escape");
    await expect(inlineInput(page)).toHaveCount(0);
    const box = boxes(page);
    await expect(box).toHaveCount(1);
    const placed = await boxOf(box.first());
    await expect.poll(() => inkNow(page)).toBeGreaterThan(SHOWS_WORDS);

    // Moved with an arrow key (ten points with Shift) ...
    await box.first().focus();
    await page.keyboard.press("Shift+ArrowRight");
    await expect.poll(async () => ((await placeOf(box))?.x ?? placed.x) - placed.x).toBeGreaterThan(5);
    // ... and resized by a handle: the box grows by what the pointer moved, and the words are
    // set again in it.
    await box.first().focus();
    const handle = page.locator("[data-handle=se]");
    await expect(handle).toBeVisible();
    const grip = await boxOf(handle);
    const [gripX, gripY] = [grip.x + grip.width / 2, grip.y + grip.height / 2];
    const moved = await boxOf(box.first());
    await page.mouse.move(gripX, gripY);
    await page.mouse.down();
    await page.mouse.move(gripX + 60, gripY + 30, { steps: 6 });
    await expect(page.locator("[data-move-preview]")).toBeVisible();
    await page.mouse.up();
    await expect
      .poll(async () => Math.round(((await placeOf(box))?.width ?? moved.width) - moved.width))
      .toBeGreaterThan(50);
    const resized = await boxOf(box.first());
    expect(Math.abs(resized.height - moved.height - 30)).toBeLessThan(4);
    await expect.poll(() => inkNow(page)).toBeGreaterThan(SHOWS_WORDS);

    // Other words in another font, in the same place: inline editor starts from what the box is.
    await box.first().focus();
    await page.keyboard.press("Enter");
    await expect(inlineInput(page)).toHaveValue("Move me");
    const bar = editorToolbar(page);
    await expect(bar.getByLabel(e.font, { exact: true })).toHaveValue(family);
    await expect(bar.getByLabel(e.size, { exact: true })).toHaveValue("24");
    const other = await fillIn(page, { words: "Moved and edited", families: ["Times New Roman", "Georgia"], blue: false });
    expect(other).not.toBe(family);
    await page.keyboard.press("Escape");
    await expect(inlineInput(page)).toHaveCount(0);
    await expect(box.first()).toHaveAccessibleName(t.textBoxSaying("Moved and edited"));
    const reworded = await boxOf(box.first());
    for (const side of ["x", "y", "width", "height"] as const) {
      expect(Math.abs(reworded[side] - resized[side]), `${side} after the words changed`).toBeLessThan(3);
    }
    await expect.poll(() => inkNow(page)).toBeGreaterThan(SHOWS_WORDS);

    // Deleted, brought back by undo; the words before are in the step before; redo goes forward.
    await box.first().focus();
    await page.keyboard.press("Delete");
    await expect(box).toHaveCount(0);
    await page.keyboard.press("Control+z");
    await expect(box).toHaveCount(1);
    await expect(box.first()).toHaveAccessibleName(t.textBoxSaying("Moved and edited"));
    await page.keyboard.press("Control+z");
    await expect(box.first()).toHaveAccessibleName(t.textBoxSaying("Move me"));
    await expect.poll(() => inkNow(page)).toBeGreaterThan(SHOWS_WORDS);
    await page.keyboard.press("Control+y");
    await expect(box.first()).toHaveAccessibleName(t.textBoxSaying("Moved and edited"));
    await page.keyboard.press("Control+y");
    await expect(box).toHaveCount(0);
    await page.keyboard.press("Control+z");
    await expect(box).toHaveCount(1);
    await expect.poll(() => inkNow(page)).toBeGreaterThan(SHOWS_WORDS);
    const ended = await boxOf(box.first());

    const copy = path.join(folder, "edited.pdf");
    await saveCopy(page, copy);
    await quit(page);

    // The copy has the box as it ended: where it is, in the words it ended with, in the font
    // chosen last, and still editable inline.
    const reopened = await launch(copy);
    await ready(reopened);
    await expect(boxes(reopened)).toHaveCount(1);
    await expect(boxes(reopened).first()).toHaveAccessibleName(t.textBoxSaying("Moved and edited"));
    const again = await boxOf(boxes(reopened).first());
    for (const side of ["x", "y", "width", "height"] as const) {
      expect(Math.abs(again[side] - ended[side]), `${side} after saving`).toBeLessThan(3);
    }
    await expect.poll(() => inkNow(reopened)).toBeGreaterThan(SHOWS_WORDS);
    await boxes(reopened).first().focus();
    await reopened.keyboard.press("Enter");
    await expect(inlineInput(reopened)).toHaveValue("Moved and edited");
    await expect(editorToolbar(reopened).getByLabel(e.font, { exact: true })).toHaveValue(other);
    await reopened.keyboard.press("Escape");
    await expect(inlineInput(reopened)).toHaveCount(0);
  } finally {
    removeFolder(folder);
  }
});

test("the keyboard stays on a text box through its edits: moved twice, changed and opened again, and the view has it once the box is deleted (#402, #719)", async ({
  launch,
}) => {
  test.setTimeout(120_000);
  const folder = folderFor("text-box-keys");
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const page = await launch(file);
    await ready(page);
    await putDown(page, await at(page, 0.3, 0.35), { words: "Keys", families: LATIN });
    const box = boxes(page).first();
    await expect(box).toHaveAccessibleName(t.textBoxSaying("Keys"));

    // Chosen once with the keyboard, and never again: each edit leaves the focus on the box, and
    // the next key goes to it.
    await box.focus();
    const start = await boxOf(box);
    await page.keyboard.press("Shift+ArrowRight");
    await expect.poll(async () => ((await placeOf(boxes(page)))?.x ?? start.x) - start.x).toBeGreaterThan(5);
    await expect(box).toBeFocused();
    const once = await boxOf(box);
    await page.keyboard.press("Shift+ArrowRight");
    await expect.poll(async () => ((await placeOf(boxes(page)))?.x ?? once.x) - once.x).toBeGreaterThan(5);
    await expect(box).toBeFocused();

    // Changed inline: Esc ends editing and gives focus back to the box, and Enter opens it again.
    const before = await firstPage(page).getAttribute("data-doc");
    await page.keyboard.press("Enter");
    await expect(inlineInput(page)).toBeVisible();
    await editorToolbar(page).getByLabel(e.size, { exact: true }).fill("30");
    await page.keyboard.press("Escape");
    await expect(inlineInput(page)).toHaveCount(0);
    await expect(firstPage(page)).not.toHaveAttribute("data-doc", before ?? "");
    await expect(page.locator("[data-page-annotations][aria-busy]")).toHaveCount(0);
    await expect(box).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(editorToolbar(page).getByLabel(e.size, { exact: true })).toHaveValue("30");
    await page.keyboard.press("Escape");
    await expect(inlineInput(page)).toHaveCount(0);
    await expect(box).toBeFocused();

    // Deleted: the box goes, and the focus goes back to the view, as with Esc, not to the window.
    await page.keyboard.press("Delete");
    await expect(boxes(page)).toHaveCount(0);
    await expect(page.getByRole("main")).toBeFocused();
  } finally {
    removeFolder(folder);
  }
});

test("a text box that was not saved comes back after a crash, from a small subset in the journal and not the font (B2-31, #719)", async ({
  launch,
}) => {
  test.setTimeout(120_000);
  const folder = folderFor("text-box-crash");
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const chinese = SCRIPTS[1]!;
    const page = await launch(file);
    await ready(page);
    await putDown(page, await at(page, 0.3, 0.55), { words: chinese.words, families: chinese.families });
    await expect(boxes(page)).toHaveCount(1);
    await expect.poll(() => inkNow(page)).toBeGreaterThan(SHOWS_WORDS);

    // The app ends with the box unsaved, as in a crash. Its journal has the words as a model
    // (hex text): the glyphs the box uses, a few kilobytes, where the font is megabytes.
    const data = dataDir(page);
    const recovery = path.join(data, "recovery");
    await expect.poll(() => (existsSync(recovery) ? readdirSync(recovery).length : 0)).toBe(1);
    const [name] = readdirSync(recovery);
    const journal = path.join(recovery, name!);
    const kept = JSON.parse(readFileSync(journal, "utf8")) as {
      textBoxes: { id: number; model: { text: string; subset: string } }[];
    };
    expect(kept.textBoxes).toHaveLength(1);
    expect(kept.textBoxes[0]!.model.text).toBe(chinese.words);
    expect(kept.textBoxes[0]!.model.subset).toMatch(/^[0-9a-f]+$/);
    expect(statSync(journal).size).toBeLessThan(600_000);
    await quit(page);

    const again = await launch(file, { dataDir: data });
    await ready(again);
    await expect(boxes(again)).toHaveCount(0);
    const banner = again.getByRole("region", { name: strings.recovery.label });
    await expect(banner).toContainText(strings.recovery.available);
    await banner.getByRole("button", { name: strings.recovery.restore }).click();
    await expect(boxes(again)).toHaveCount(1);
    await expect(boxes(again).first()).toHaveAccessibleName(t.textBoxSaying(chinese.words));
    await expect.poll(() => inkNow(again)).toBeGreaterThan(SHOWS_WORDS);
    // Undone and made again from the model the journal had.
    await again.keyboard.press("Control+z");
    await expect(boxes(again)).toHaveCount(0);
    await again.keyboard.press("Control+y");
    await expect(boxes(again)).toHaveCount(1);
    await expect.poll(() => inkNow(again)).toBeGreaterThan(SHOWS_WORDS);

    const copy = path.join(folder, "restored.pdf");
    await saveCopy(again, copy);
    await quit(again);
    const reopened = await launch(copy);
    await ready(reopened);
    await expect(boxes(reopened)).toHaveCount(1);
    await expect.poll(() => inkNow(reopened)).toBeGreaterThan(SHOWS_WORDS);
  } finally {
    removeFolder(folder);
  }
});

test("a font that cannot be used says why, inline editing stays open, and another font commits (B2-31, #719)", async ({
  launch,
}) => {
  test.setTimeout(120_000);
  const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
  await ready(page);
  // This time by the context menu, where the box is to be.
  const [x, y] = await at(page, 0.3, 0.55);
  await page.mouse.click(x, y, { button: "right" });
  await page.getByRole("menuitem", { name: t.addTextBox }).click();
  const words = inlineInput(page);
  await expect(words).toBeVisible();
  const bar = editorToolbar(page);
  const font = bar.getByLabel(e.font, { exact: true });
  const problem = bar.getByRole("alert");

  // Chinese words, in a font that has no Chinese.
  await words.fill("中文字");
  await font.selectOption({ label: await offered(bar, LATIN) });
  await page.keyboard.press("Escape");
  await expect(problem).toHaveText(strings.error.messages.fontMissingGlyphs);
  await expect(words).toBeVisible();
  await expect(boxes(page)).toHaveCount(0);

  // A variable font, and a colour font: neither can be drawn from a subset.
  await words.fill("Hello");
  await font.selectOption({ label: await offered(bar, ["Bahnschrift"]) });
  await expect(problem).toHaveText(strings.error.messages.fontVariable);
  await font.selectOption({ label: await offered(bar, ["Segoe UI Emoji"]) });
  await expect(problem).toHaveText(strings.error.messages.fontColour);
  await expect(boxes(page)).toHaveCount(0);

  // The words that were refused, in a font that has them: Esc commits.
  await font.selectOption({ label: await offered(bar, LATIN) });
  await page.keyboard.press("Escape");
  await expect(words).toHaveCount(0);
  await expect(boxes(page)).toHaveCount(1);
  await expect(boxes(page).first()).toHaveAccessibleName(t.textBoxSaying("Hello"));
});

const REGISTRY_FONTS = "HKCU\\Software\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";

/**
 * Registering a font for the current user changes this computer's settings. It is done only on a
 * runner that is thrown away after the run (CI), or when asked for with `E2E_REGISTER_FONT=1`.
 */
const MAY_REGISTER_FONTS = !!process.env.CI || process.env.E2E_REGISTER_FONT === "1";
const NOT_ON_THIS_COMPUTER =
  "it registers a font for the current user: only on a runner that is thrown away (E2E_REGISTER_FONT=1 asks for it)";

/** Lists `file` as a font installed for the current user, the way Windows lists one, under `family`. */
function registerFont(family: string, file: string) {
  const value = `${family} (TrueType)`;
  execFileSync("reg", ["add", REGISTRY_FONTS, "/v", value, "/t", "REG_SZ", "/d", file, "/f"], { stdio: "ignore" });
}

/** Takes `family` off the list again; nothing to do if it was never on it. */
function unregisterFont(family: string) {
  try {
    execFileSync("reg", ["delete", REGISTRY_FONTS, "/v", `${family} (TrueType)`, "/f"], { stdio: "ignore" });
  } catch {
    // It was never there, or is gone already.
  }
}

/** Arial, which every Windows has, to make the fonts below from. */
const ARIAL = path.join(process.env.SystemRoot ?? "C:\\Windows", "Fonts", "arial.ttf");

function altered(font: Buffer, changes: { fsType?: number; hideOutlines?: boolean }): Buffer {
  const copy = Buffer.from(font);
  const tables = copy.readUInt16BE(4);
  for (let index = 0; index < tables; index++) {
    const record = 12 + index * 16;
    const tag = copy.toString("latin1", record, record + 4);
    if (tag === "OS/2" && changes.fsType !== undefined) {
      copy.writeUInt16BE(changes.fsType, copy.readUInt32BE(record + 8) + 8);
    }
    if (tag === "glyf" && changes.hideOutlines) copy.write("gxyf", record, "latin1");
  }
  return copy;
}

test("a font whose licence does not allow a copy in the file, or that is not TrueType, is refused, and one for viewing only is used with a note (B2-31, #719)", async ({
  launch,
}) => {
  test.skip(!MAY_REGISTER_FONTS, NOT_ON_THIS_COMPUTER);
  test.setTimeout(120_000);
  const folder = folderFor("text-box-licence");
  const fonts = [
    { family: "E2E Restricted Font", changes: { fsType: 0x0002 }, refused: strings.error.messages.fontNotEmbeddable },
    { family: "E2E Outline Font", changes: { hideOutlines: true }, refused: strings.error.messages.fontNotTrueType },
    { family: "E2E Preview Font", changes: { fsType: 0x0004 }, refused: null },
  ];
  try {
    const arial = readFileSync(ARIAL);
    for (const { family, changes } of fonts) {
      const file = path.join(folder, `${family.replaceAll(" ", "-")}.ttf`);
      writeFileSync(file, altered(arial, changes));
      registerFont(family, file);
    }
    const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
    await ready(page);
    await page.getByRole("button", { name: t.textBox, exact: true }).click();
    await page.mouse.click(...(await at(page, 0.3, 0.5)));
    await expect(inlineInput(page)).toBeVisible();
    await inlineInput(page).fill("Licence");
    const bar = editorToolbar(page);
    for (const { family, refused } of fonts.filter(({ refused }) => refused !== null)) {
      await bar.getByLabel(e.font, { exact: true }).selectOption({ label: family });
      await expect(bar.getByRole("alert"), family).toHaveText(refused!);
      await expect(boxes(page)).toHaveCount(0);
    }
    // A licence for viewing and printing only: the box is made, and the status bar says what that means.
    const preview = fonts[2]!.family;
    await bar.getByLabel(e.font, { exact: true }).selectOption({ label: preview });
    await page.keyboard.press("Escape");
    await expect(inlineInput(page)).toHaveCount(0);
    await expect(boxes(page)).toHaveCount(1);
    await expect(page.getByRole("contentinfo")).toContainText(t.textBoxViewOnly(preview));
  } finally {
    for (const { family } of fonts) unregisterFont(family);
    removeFolder(folder);
  }
});

test("a font that is gone says so: while typing, and in a document that came from another computer (B2-31, #719)", async ({
  launch,
}) => {
  test.skip(!MAY_REGISTER_FONTS, NOT_ON_THIS_COMPUTER);
  test.setTimeout(180_000);
  const folder = folderFor("text-box-gone");
  const family = "E2E Vanishing Font";
  try {
    const font = path.join(folder, "vanishing.ttf");
    copyFileSync(ARIAL, font);
    registerFont(family, font);

    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const page = await launch(file);
    await ready(page);
    await putDown(page, await at(page, 0.3, 0.35), { words: "Gone", families: [family] });
    await expect(boxes(page)).toHaveCount(1);
    await expect.poll(() => inkNow(page)).toBeGreaterThan(SHOWS_WORDS);
    const away = path.join(folder, "from-another-computer.pdf");
    await saveCopy(page, away);

    // The font goes while the inline editor is open: the list it shows is no longer true.
    await page.getByRole("button", { name: t.textBox, exact: true }).click();
    await page.mouse.click(...(await at(page, 0.3, 0.7)));
    await expect(inlineInput(page)).toBeVisible();
    await inlineInput(page).fill("Second");
    const bar = editorToolbar(page);
    await bar.getByLabel(e.font, { exact: true }).selectOption({ label: family });
    rmSync(font);
    await page.keyboard.press("Escape");
    await expect(bar.getByRole("alert")).toHaveText(strings.error.messages.fontUnavailable);
    await expect(inlineInput(page)).toBeVisible();
    await expect(boxes(page)).toHaveCount(1);
    await quit(page);

    // The document is opened where the font is not installed.
    unregisterFont(family);
    const there = await launch(away);
    await ready(there);
    await expect(boxes(there)).toHaveCount(1);
    await expect.poll(() => inkNow(there)).toBeGreaterThan(SHOWS_WORDS);
    const opened = await firstPage(there).getAttribute("data-doc");
    await boxes(there).first().focus();
    await there.keyboard.press("Enter");
    await expect(inlineInput(there)).toBeVisible();
    const editBar = editorToolbar(there);
    const menu = editBar.getByLabel(e.font, { exact: true });
    await expect(menu).toHaveValue("");
    expect(await menu.locator("option:checked").textContent()).toBe(e.fontMissing(family));
    // The size needs no font: the box is set again from what the file has.
    await editBar.getByLabel(e.size, { exact: true }).fill("30");
    await there.keyboard.press("Escape");
    await expect(inlineInput(there)).toHaveCount(0);
    await expect(firstPage(there)).not.toHaveAttribute("data-doc", opened ?? "");
    await expect(there.locator("[data-page-annotations][aria-busy]")).toHaveCount(0);
    // New words need a font: the toolbar says to choose another, and a font that is here does it.
    await expect(boxes(there)).toHaveCount(1);
    await boxes(there).first().focus();
    await expect(boxes(there).first()).toBeFocused();
    await there.keyboard.press("Enter");
    await expect(editorToolbar(there).getByLabel(e.size, { exact: true })).toHaveValue("30");
    await inlineInput(there).fill("Found");
    await there.keyboard.press("Escape");
    await expect(editorToolbar(there).getByRole("alert")).toHaveText(e.needFont);
    await fillIn(there, { words: "Found", families: LATIN, size: 30, blue: false });
    await there.keyboard.press("Escape");
    await expect(inlineInput(there)).toHaveCount(0);
    await expect(boxes(there).first()).toHaveAccessibleName(t.textBoxSaying("Found"));
  } finally {
    unregisterFont(family);
    removeFolder(folder);
  }
});
