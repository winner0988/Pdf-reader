// English UI (ADR 0029, #683) in the real app:
// Starts with English UI setting and verifies the English string tables,
// language attribute, key screens, and accessibility (axe).
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

import type { Page } from "@playwright/test";

import { strings as enStrings } from "../../src/i18n/en";
import { corpus, expect, test } from "./app";

const axeSource = readFileSync(createRequire(import.meta.url).resolve("axe-core/axe.min.js"), "utf8");

const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"];
const APPLICATION = { "page-has-heading-one": "an application window, not a page of text" };
const TABS = { region: "axe does not look inside a labelled tab panel for its landmarks" };

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

async function problems(page: Page, skip: Record<string, string> = {}): Promise<string[]> {
  await page.waitForFunction(() => document.getAnimations().length === 0);
  if ((await page.evaluate(() => typeof (window as unknown as { axe?: Axe }).axe)) === "undefined") {
    await page.evaluate(`${axeSource}\n;void 0`);
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

test("starts with English interface, displays English UI, and passes accessibility checks", async ({ launch }) => {
  const page = await launch(corpus("benign/single-page.pdf"), { systemLanguage: "en" });
  await page.emulateMedia({ colorScheme: "light" });

  // HTML lang attribute reflects English setting
  await expect(page.locator("html")).toHaveAttribute("lang", "en");

  // Page canvas renders in English
  await expect(page.getByRole("img", { name: enStrings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Toolbar buttons and zoom dropdown have English tooltips / accessible names
  await expect(page.getByRole("button", { name: enStrings.toolbar.more })).toBeVisible();
  await expect(page.getByRole("combobox", { name: enStrings.toolbar.zoomLevel })).toBeVisible();
  await expect(page.getByRole("button", { name: enStrings.toolbar.zoomIn })).toBeVisible();

  // Accessibility check on main reader view
  expect(await problems(page, { ...APPLICATION, ...TABS })).toEqual([]);

  // Open Settings dialog
  await page.getByRole("button", { name: enStrings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: enStrings.menu.settings }).click();

  const settingsDialog = page.getByRole("dialog", { name: enStrings.settings.title });
  await expect(settingsDialog).toBeVisible();
  await expect(settingsDialog.getByRole("heading", { name: enStrings.settings.languageTitle })).toBeVisible();
  await expect(settingsDialog.getByRole("radio", { name: enStrings.settings.languageSystem(enStrings.settings.languageEn) })).toBeChecked();

  // Accessibility check on settings dialog
  expect(await problems(page, APPLICATION)).toEqual([]);

  await page.keyboard.press("Escape");
  await expect(settingsDialog).not.toBeVisible();
});
