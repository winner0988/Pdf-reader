// Where search finds a word on screen, for specs that then click or drag there. Going to a hit
// scrolls it a third of the way down the view (docs/ux/screen-map.md, section 3), so a hit's box is
// read only once the search has gone to it and the box has stopped moving (#325).
import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { expect } from "./app";

export type Box = { x: number; y: number; width: number; height: number };

/** Where the only hit of the search is on screen, once the view has scrolled to it. */
export async function hitOnScreen(page: Page): Promise<Box> {
  await expect(page.locator("[data-highlights] polygon[data-current]")).toHaveCount(1);
  // The hit's fill: the outline of the current hit is drawn with a stroke, which widens its box.
  const hit = page.locator("[data-highlights] polygon:not([data-current])");
  await expect(hit).toHaveCount(1);
  const seen: { box: Box | null } = { box: null };
  await expect
    .poll(async () => {
      const box = await hit.boundingBox();
      const settled = box !== null && JSON.stringify(box) === JSON.stringify(seen.box);
      seen.box = box;
      return settled;
    })
    .toBe(true);
  return seen.box!;
}

/** Searches for `query`, whose only hit it expects, and returns where the hit is on screen. */
export async function findOnScreen(page: Page, query: string): Promise<Box> {
  await page.keyboard.press("Control+f");
  const input = page.getByRole("textbox", { name: strings.search.placeholder });
  await input.fill(query);
  await input.press("Enter");
  return hitOnScreen(page);
}
