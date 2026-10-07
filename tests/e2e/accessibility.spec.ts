// Accessibility (#192) in the real app: what the jsdom checks of #187 cannot measure, colour
// contrast above all, and the rest of axe's rules on what WebView2 really lays out, in the light
// and in the dark theme. axe-core is put in the page by the test (CDP's `Runtime.evaluate`, which
// the page's CSP does not apply to); the app itself carries none of it.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, launchAgain, test } from "./app";

const axeSource = readFileSync(createRequire(import.meta.url).resolve("axe-core/axe.min.js"), "utf8");

/** WCAG 2.0 and 2.1 level A and AA, and axe's own best practices; colour contrast is measured. */
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"];

type Axe = {
  run(
    context: Document,
    options: object,
  ): Promise<{
    violations: {
      id: string;
      impact: string | null;
      help: string;
      nodes: { target: unknown[]; any: { message: string; relatedNodes?: { target: unknown[] }[] }[] }[];
    }[];
  }>;
};

/** What axe finds wrong with the page as it is now, one line for each element. */
async function problems(page: Page, skip: Record<string, string> = {}): Promise<string[]> {
  // Colours that are still changing (a theme that was just switched) are not what the user sees.
  await page.waitForFunction(() => document.getAnimations().length === 0);
  // As an expression, which CDP evaluates whatever the page's CSP says about `eval`.
  if ((await page.evaluate(() => typeof (window as unknown as { axe?: Axe }).axe)) === "undefined") {
    await page.evaluate(`${axeSource}
;void 0`);
  }
  return page.evaluate(
    async ({ tags, skipped }) => {
      const axe = (window as unknown as { axe: Axe }).axe;
      const results = await axe.run(document, {
        runOnly: { type: "tag", values: tags },
        rules: Object.fromEntries(skipped.map((id) => [id, { enabled: false }])),
      });
      return results.violations.flatMap((violation) =>
        violation.nodes.map(
          (node) =>
            `${violation.id} (${violation.impact ?? "?"}): ${violation.help}: ${node.target.join(" ")}: ${node.any[0]?.message ?? ""}` +
            (node.any[0]?.relatedNodes?.length
              ? ` [${node.any[0].relatedNodes.map((related) => related.target.join(" ")).join(" | ")}]`
              : ""),
        ),
      );
    },
    { tags: TAGS, skipped: Object.keys(skip) },
  );
}

/** A menu is a pop-up: outside the page's landmarks by design. */
const POPUP = { region: "a menu is a pop-up, outside the page's landmarks by design" };

/** A rule for web pages; this is an application window, whose title is the window's. */
const APPLICATION = { "page-has-heading-one": "an application window, not a page of text" };

/**
 * What axe says about a window that has tabs, and is not a problem of the window:
 * - `region`: axe counts a labelled element (the tab panel, which `aria-labelledby` names after its
 *   tab) as content and does not look inside it for the landmarks it holds; the landmarks are
 *   there, and no text is outside them (checked by walking the page's text);
 * - `aria-required-children`: the close buttons of the tabs are in the tablist, which needs a
 *   decision: #188.
 */
const TABS = {
  region: "axe does not look inside a labelled tab panel for its landmarks",
  "aria-required-children": "the close buttons of the tabs are in the tablist: #188",
};

async function chooseFromMenu(page: Page, name: string) {
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: new RegExp(`^${name}`) }).click();
}

const THEMES = ["light", "dark"] as const;

/** Runs `check` with the system in each theme, which the app follows, as it is on the page now. */
async function inBothThemes(page: Page, check: (theme: (typeof THEMES)[number]) => Promise<void>) {
  for (const theme of THEMES) {
    await page.emulateMedia({ colorScheme: theme });
    await expect(page.locator("html"), theme).toHaveClass(theme === "dark" ? /\bdark\b/ : /^(?!.*\bdark\b)/);
    await check(theme);
  }
}

const noProblems = async (page: Page, theme: string, skip: Record<string, string> = {}) => {
  const tabs = (await page.getByRole("tablist").count()) > 0;
  expect(await problems(page, { ...APPLICATION, ...(tabs ? TABS : {}), ...skip }), theme).toEqual([]);
};

test("the window with nothing open, and the menu", async ({ launch }) => {
  const page = await launch();
  await inBothThemes(page, (theme) => noProblems(page, theme));

  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await expect(page.getByRole("menu")).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, theme, POPUP));
});

test("a document with an outline, its thumbnails and its search bar", async ({ launch }) => {
  const page = await launch(corpus("benign/outline-3-levels.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  await inBothThemes(page, (theme) => noProblems(page, theme));

  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  await expect(page.getByRole("option", { name: strings.canvas.page(2) })).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, theme));

  await page.getByRole("button", { name: strings.toolbar.search }).click();
  await expect(page.getByRole("search")).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, theme));
});

test("a document with something active in it: the banner and its details", async ({ launch }) => {
  const page = await launch(corpus("malicious/openaction-js.pdf"));
  await expect(page.getByRole("region", { name: strings.banner.label })).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, theme));

  await page.getByRole("button", { name: strings.banner.details }).click();
  await expect(page.getByText(strings.banner.detailsNote)).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, theme));
});

test("a signed document: the signature banner and its panel", async ({ launch }) => {
  const page = await launch(corpus("benign/signed.pdf"));
  await expect(page.getByRole("region", { name: strings.signatures.label })).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, theme));

  await page.getByRole("button", { name: strings.signatures.details }).click();
  await expect(page.getByRole("complementary", { name: strings.signatures.detailsTitle })).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, theme));
});

test("a document with a form, and one that asks for a password", async ({ launch }) => {
  const form = await launch(corpus("benign/form-fields.pdf"));
  await expect(form.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  await expect(form.locator("[data-page-fields] [data-field]").first()).toBeVisible();
  await inBothThemes(form, (theme) => noProblems(form, theme));

  // The app runs once: the second file opens as a tab of the same window, and asks for its password.
  launchAgain(corpus("benign/encrypted-aes256.pdf"));
  await expect(form.getByRole("form", { name: strings.password.title })).toBeVisible();
  await inBothThemes(form, (theme) => noProblems(form, theme));
});

test("the dialogs of the menu", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  const dialogs: [string, string][] = [
    [strings.menu.settings, strings.settings.title],
    [strings.menu.shortcuts, strings.shortcuts.title],
    [strings.menu.about, strings.about.title],
    [strings.menu.export, strings.export.title],
    [strings.menu.privacyExport, strings.privacyExport.title],
    [strings.menu.encryptCopy, strings.encryption.title],
  ];
  for (const [item, title] of dialogs) {
    await chooseFromMenu(page, item);
    await expect(page.getByRole("dialog", { name: title })).toBeVisible();
    await inBothThemes(page, (theme) => noProblems(page, `${title}, ${theme}`));
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
  }
});

test("the system's setting to reduce motion is respected", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  /** The longest time any element takes to change or animate, in seconds. */
  const longest = () =>
    page.evaluate(() => {
      const seconds = (list: string) =>
        list.split(",").map((time) => (time.trim().endsWith("ms") ? parseFloat(time) / 1000 : parseFloat(time)));
      return Math.max(
        ...[...document.querySelectorAll("*")].flatMap((element) => {
          const style = getComputedStyle(element);
          return [...seconds(style.transitionDuration), ...seconds(style.animationDuration)];
        }),
      );
    });
  await page.emulateMedia({ reducedMotion: "no-preference" });
  expect(await longest()).toBeGreaterThan(0.05);
  await page.emulateMedia({ reducedMotion: "reduce" });
  expect(await longest()).toBeLessThan(0.001);
});
