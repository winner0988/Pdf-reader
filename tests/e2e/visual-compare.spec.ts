// Visual comparison between two open tabs (B5-14, #451).
// Comparing two tabs page-by-page side-by-side or overlaid, marking visual differences.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, launchAgain, test } from "./app";

const t = strings.compare.visual;

const axeSource = readFileSync(createRequire(import.meta.url).resolve("axe-core/axe.min.js"), "utf8");
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

const THEMES = ["light", "dark"] as const;

async function inBothThemes(page: Page, check: (theme: (typeof THEMES)[number]) => Promise<void>) {
  for (const theme of THEMES) {
    await page.emulateMedia({ colorScheme: theme });
    await expect(page.locator("html"), theme).toHaveClass(theme === "dark" ? /\bdark\b/ : /^(?!.*\bdark\b)/);
    await check(theme);
  }
}

const TABS = { region: "axe does not look inside a labelled tab panel for its landmarks" };

test.describe("visual comparison (B5-14, #451)", () => {
  test("compares two tabs with an image difference, marks differences, and passes axe in light and dark", async ({
    launch,
  }) => {
    // Launch first tab with compare-picture-a.pdf
    const page = await launch(corpus("benign/compare-picture-a.pdf"));
    await expect(page.getByRole("contentinfo")).toContainText("compare-picture-a.pdf");

    // Launch second tab with compare-picture-b.pdf
    launchAgain(corpus("benign/compare-picture-b.pdf"));
    const tabs = page.getByRole("tablist", { name: strings.tabs.label }).getByRole("tab");
    await expect(tabs).toHaveCount(2);
    await expect(page.getByRole("contentinfo")).toContainText("compare-picture-b.pdf");

    // Open More menu and choose visual compare
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.compareVisual}`) }).click();

    // Dialog appears
    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toBeVisible();

    // Select target tab
    const select = dialog.getByLabel(t.chooseTab);
    await expect(select).toBeVisible();

    // Click start compare
    await dialog.getByRole("button", { name: t.start }).click();

    // Status announces the difference found
    const status = dialog.getByRole("status");
    await expect(status).toBeVisible();
    await expect(status).toContainText("第 1 處，共 1 處");

    // Both images are visible
    const imgA = dialog.getByRole("img", { name: new RegExp("compare-picture-a\\.pdf") });
    const imgB = dialog.getByRole("img", { name: new RegExp("compare-picture-b\\.pdf") });
    await expect(imgA).toBeVisible();
    await expect(imgB).toBeVisible();

    // Verify accessibility in light and dark themes
    await inBothThemes(page, async (theme) => {
      const violations = await problems(page, TABS);
      expect(violations, `axe violations in ${theme} theme`).toEqual([]);
    });

    // Close dialog
    await dialog.getByRole("button", { name: t.close }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });

  test("compares two visually identical documents and shows no differences", async ({ launch }) => {
    // Launch first tab with compare-picture-a.pdf
    const page = await launch(corpus("benign/compare-picture-a.pdf"));
    await expect(page.getByRole("contentinfo")).toContainText("compare-picture-a.pdf");

    // Launch second tab with compare-same-as-a.pdf
    launchAgain(corpus("benign/compare-same-as-a.pdf"));
    const tabs = page.getByRole("tablist", { name: strings.tabs.label }).getByRole("tab");
    await expect(tabs).toHaveCount(2);
    await expect(page.getByRole("contentinfo")).toContainText("compare-same-as-a.pdf");

    // Open More menu and choose visual compare
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.compareVisual}`) }).click();

    // Dialog appears
    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toBeVisible();

    // Click start compare
    await dialog.getByRole("button", { name: t.start }).click();

    // Status reports no difference
    const status = dialog.getByRole("status");
    await expect(status).toBeVisible();
    await expect(status).toContainText(t.noDiff);

    // Close dialog
    await dialog.getByRole("button", { name: t.close }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });
});
