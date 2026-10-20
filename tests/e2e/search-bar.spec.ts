// The search bar and what is above and beside the page (#281). The bar floats at the top right of
// the page area (docs/ux/screen-map.md, section 3): under the banners, and to the left of a details
// panel when one is open. With it open, every button of a banner and of its panel can still be
// pressed. Where things are on screen is known only to the real WebView (jsdom lays nothing out),
// so the boxes are compared here.
import type { Locator } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

type Box = { x: number; y: number; width: number; height: number };

async function boxOf(locator: Locator): Promise<Box> {
  const box = await locator.boundingBox();
  if (!box) throw new Error("it is not on screen");
  return box;
}

const overlap = (a: Box, b: Box) =>
  a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

/** The names of the `buttons` that are under (or partly under) `bar`: none, when all can be pressed. */
async function covered(bar: Locator, buttons: Locator): Promise<string[]> {
  const barBox = await boxOf(bar);
  const under: string[] = [];
  for (const button of await buttons.all()) {
    if (overlap(await boxOf(button), barBox)) {
      under.push((await button.getAttribute("aria-label")) ?? (await button.innerText()));
    }
  }
  return under;
}

/** The words of a banner and of its details panel; the two kinds opened here have the same ones. */
type Words = { label: string; details: string; dismiss: string; detailsTitle: string; detailsClose: string };

const BANNERS: Array<{ name: string; file: string; words: Words }> = [
  { name: "blocked content", file: "malicious/openaction-js.pdf", words: strings.banner },
  { name: "signatures", file: "benign/signed.pdf", words: strings.signatures },
];

for (const { name, file, words } of BANNERS) {
  test(`the search bar floats under the ${name} banner and beside its details, so no button is covered (#281)`, async ({
    launch,
  }) => {
    const page = await launch(corpus(file));
    const banner = page.getByRole("region", { name: words.label });
    await expect(banner).toBeVisible();
    await page.keyboard.press("Control+f");
    const bar = page.getByRole("search", { name: strings.search.label });
    await expect(bar).toBeVisible();
    const area = page.getByRole("main", { name: strings.canvas.label });

    /** The bar is inside the page area, which begins under the banner and ends where a panel begins. */
    const inPageArea = async () => {
      const barBox = await boxOf(bar);
      const areaBox = await boxOf(area);
      expect(barBox.y, "the bar's top is in the page area").toBeGreaterThanOrEqual(areaBox.y);
      expect(barBox.x, "the bar's left edge is in the page area").toBeGreaterThanOrEqual(areaBox.x);
      expect(barBox.x + barBox.width, "the bar's right edge is in the page area").toBeLessThanOrEqual(
        areaBox.x + areaBox.width,
      );
    };

    // The bar is under the banner, so none of the banner's buttons is covered ...
    await inPageArea();
    expect(await covered(bar, banner.getByRole("button")), "the banner's buttons under the bar").toEqual([]);

    // ... the details button can be pressed, and the panel it opens is beside the page area, so
    // none of its buttons is covered either ...
    await banner.getByRole("button", { name: words.details }).click();
    const panel = page.getByRole("complementary", { name: words.detailsTitle });
    await expect(panel).toBeVisible();
    await inPageArea();
    expect(await covered(bar, panel.getByRole("button")), "the panel's buttons under the bar").toEqual([]);
    await panel.getByRole("button", { name: words.detailsClose }).click();
    await expect(panel).toHaveCount(0);

    // ... and the banner's last button, at the right where the bar used to be, can be pressed: the
    // bar then stays at the top of the page area, which has grown to where the banner was.
    await banner.getByRole("button", { name: words.dismiss }).click();
    await expect(banner).toHaveCount(0);
    await expect(bar).toBeVisible();
    await inPageArea();
  });
}
