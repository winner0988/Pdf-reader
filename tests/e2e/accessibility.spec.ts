// Accessibility (#192) in the real app: what the jsdom checks of #187 cannot measure, colour
// contrast above all, and the rest of axe's rules on what WebView2 really lays out, in the light
// and in the dark theme. axe-core is put in the page by the test (CDP's `Runtime.evaluate`, which
// the page's CSP does not apply to); the app itself carries none of it.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, dataDir, expect, launchAgain, quit, test } from "./app";
import { leaveNothing, SAMPLE } from "./passwords";

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
    [strings.menu.compressImages, strings.compress.title],
    [strings.menu.batch, strings.batch.title],
    [strings.menu.createPdf, strings.create.title],
    // Only asked about: closed before anything is written to the corpus file (B3-05).
    [strings.menu.markSensitive, strings.sensitive.markTitle],
  ];
  for (const [item, title] of dialogs) {
    await chooseFromMenu(page, item);
    await expect(page.getByRole("dialog", { name: title })).toBeVisible();
    await inBothThemes(page, (theme) => noProblems(page, `${title}, ${theme}`));
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
  }
});

test("the text box dialog, with a problem shown, and the toolbar of a text box on the page (B2-31)", async ({ launch }) => {
  const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
  const sheet = page.getByRole("img", { name: strings.canvas.page(1) });
  await expect(sheet).toHaveAttribute("data-state", "ready");
  const t = strings.annotations;
  const dialog = page.getByRole("dialog", { name: t.textBoxDialog.addTitle });

  // The tool is on, and a click on the page (all of it on the screen) asks what the box says.
  await page.keyboard.press("Control+0");
  await expect.poll(async () => (await sheet.boundingBox())?.height ?? Infinity).toBeLessThan(700);
  await page.getByRole("button", { name: t.textBox, exact: true }).click();
  const area = (await sheet.boundingBox())!;
  await page.mouse.click(area.x + area.width * 0.5, area.y + area.height * 0.6);
  await expect(dialog).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `new text box, ${theme}`));

  // Saved with nothing in it: the problem is said in the dialog.
  await dialog.getByRole("button", { name: t.textBoxDialog.save, exact: true }).click();
  await expect(dialog.getByRole("alert")).toHaveText(t.textBoxDialog.empty);
  await inBothThemes(page, (theme) => noProblems(page, `text box with a problem, ${theme}`));

  // A box on the page, chosen: its outline, handles and small toolbar.
  await dialog.getByLabel(t.textBoxDialog.text, { exact: true }).fill("Contrast");
  await dialog.getByRole("button", { name: t.textBoxDialog.save, exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.locator("[data-page-annotations] [data-annotation=textBox]").first().focus();
  await expect(page.getByRole("toolbar", { name: t.textBoxSaying("Contrast") })).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `text box chosen, ${theme}`));

  // The dialog again, for the box: it starts from what the box is.
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", { name: t.textBoxDialog.editTitle })).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `text box changed, ${theme}`));
});

test("the password form that can remember, the notice of a remembered password, and the remembered passwords in the settings (B2-26)", async ({
  launch,
}) => {
  const sample = SAMPLE;
  const t = strings.password;
  const s = strings.settings.passwords;
  let page = await launch(sample);
  const data = dataDir(page);
  try {
    // The form, with the box that asks to remember (off) and the note under it.
    await expect(page.getByRole("heading", { name: t.title })).toBeVisible();
    await expect(page.getByRole("checkbox", { name: new RegExp(t.remember) })).not.toBeChecked();
    await inBothThemes(page, (theme) => noProblems(page, `password form, ${theme}`));
    await page.getByRole("checkbox", { name: new RegExp(t.remember) }).check();
    const field = page.getByLabel(t.label, { exact: true });
    await field.fill("user");
    await field.press("Enter");
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
    await quit(page);

    // The next start: the file opens by itself, and the notice says so.
    page = await launch(sample, { dataDir: data });
    const notice = page.getByRole("region", { name: t.recalled.label });
    await expect(notice).toBeVisible();
    await inBothThemes(page, (theme) => noProblems(page, `notice of a remembered password, ${theme}`));

    // The settings count them, and ask before forgetting all of them.
    await chooseFromMenu(page, strings.menu.settings);
    const dialog = page.getByRole("dialog", { name: strings.settings.title });
    await expect(dialog).toContainText(/已記住 \d+ 份/);
    await inBothThemes(page, (theme) => noProblems(page, `settings with remembered passwords, ${theme}`));
    await dialog.getByRole("button", { name: s.forgetAll }).click();
    await expect(dialog.getByRole("group", { name: s.confirmTitle })).toBeVisible();
    await inBothThemes(page, (theme) => noProblems(page, `the question before forgetting all, ${theme}`));
    // Not this one: only this test's own password is forgotten, from the notice.
    await dialog.getByRole("button", { name: s.cancel }).click();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await notice.getByRole("button", { name: t.recalled.forget }).click();
    await expect(notice).toHaveCount(0);
    await quit(page);
  } finally {
    // Whatever happened, the sample's password is not left in the Credential Manager.
    await leaveNothing(launch, page, data);
  }
});

test("the licences of what the app is made of, a view of the About dialog", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  await chooseFromMenu(page, strings.menu.about);
  await page
    .getByRole("dialog", { name: strings.about.title })
    .getByRole("button", { name: strings.about.thirdParty })
    .click();
  // The text is a chunk of the app that is loaded when it is asked for.
  await expect(page.getByRole("region", { name: strings.licenses.textLabel })).toContainText(
    "GNU AFFERO GENERAL PUBLIC LICENSE",
  );
  await inBothThemes(page, (theme) => noProblems(page, `third-party licences, ${theme}`));
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
