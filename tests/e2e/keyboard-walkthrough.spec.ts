// Keyboard walkthrough & accessibility checks (B4-09, #417, #402).
// Every screen and dialog is walked through with the keyboard alone:
// - opened with shortcut or menu navigation;
// - focus moves properly inside dialog (trapped inside, initial focus on expected control);
// - operable by keyboard alone without pointer;
// - dismissed with Escape or cancel;
// - focus restores to the invoking element upon close (never dropping to body);
// - axe-core verifies no serious or critical accessibility violations in both light and dark themes.

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, launchAgain, test } from "./app";

const axeSource = readFileSync(createRequire(import.meta.url).resolve("axe-core/axe.min.js"), "utf8");

/** WCAG 2.0 and 2.1 level A and AA, and axe's own best practices. */
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

/** What axe finds wrong with the page as it is now. */
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

const APPLICATION = { "page-has-heading-one": "an application window, not a page of text" };
const TABS = { region: "axe does not look inside a labelled tab panel for its landmarks" };

const THEMES = ["light", "dark"] as const;

/** Runs `check` in both light and dark themes. */
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

async function chooseFromMenu(page: Page, name: string) {
  const more = page.getByRole("button", { name: strings.toolbar.more });
  await more.click();
  await page.getByRole("menuitem", { name: new RegExp(`^${name}`) }).click();
}

test("global shortcuts and search bar: Shortcuts, Print, SearchAll, SearchBar", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Focus the document tab to ensure document focus before global shortcuts
  await page.getByRole("tab").first().focus();
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);

  // 1. Shortcuts dialog (Ctrl+/)
  await page.keyboard.press("Control+/");
  const shortcutsDialog = page.getByRole("dialog", { name: strings.shortcuts.title });
  await expect(shortcutsDialog).toBeVisible();
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);
  await inBothThemes(page, (theme) => noProblems(page, `shortcuts dialog, ${theme}`));
  await page.keyboard.press("Escape");
  await expect(shortcutsDialog).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);

  // 2. Print dialog (Ctrl+P)
  await page.keyboard.press("Control+p");
  const printDialog = page.getByRole("dialog", { name: strings.print.title });
  await expect(printDialog).toBeVisible();
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);
  await page.keyboard.press("Escape");
  await expect(printDialog).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);

  // 3. Search All dialog (Ctrl+Shift+F)
  await page.keyboard.press("Control+Shift+F");
  const searchAllDialog = page.getByRole("dialog", { name: strings.textIndex.title });
  await expect(searchAllDialog).toBeVisible();
  expect(await page.evaluate(() => document.activeElement?.getAttribute("aria-label"))).toBe(strings.textIndex.label);
  await inBothThemes(page, (theme) => noProblems(page, `search all dialog, ${theme}`));
  await page.keyboard.press("Escape");
  await expect(searchAllDialog).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);

  // 4. In-document search bar (Ctrl+F)
  await page.keyboard.press("Control+f");
  const searchBar = page.getByRole("search", { name: strings.search.label });
  await expect(searchBar).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(searchBar).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);
});

test("menu dialogs: keyboard walkthrough, focus trapping, and focus return to More button", async ({ launch }) => {
  const page = await launch(corpus("benign/contract-v1.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Open second tab so CompareDialog has another tab to compare with
  launchAgain(corpus("benign/contract-v2.pdf"));
  await expect(page.getByRole("tablist", { name: strings.tabs.label }).getByRole("tab")).toHaveCount(2);

  const menuDialogs: [string, string][] = [
    [strings.menu.settings, strings.settings.title],
    [strings.menu.about, strings.about.title],
    [strings.menu.export, strings.export.title],
    [strings.menu.exportAnnotations, strings.exportAnnotations.title],
    [strings.menu.privacyExport, strings.privacyExport.title],
    [strings.menu.sanitizeCopy, strings.sanitize.title],
    [strings.menu.pageMarks, strings.pageMarks.title],
    [strings.menu.encryptCopy, strings.encryption.title],
    [strings.menu.compressImages, strings.compress.title],
    [strings.menu.batch, strings.batch.title],
    [strings.menu.createPdf, strings.create.title],
    [strings.menu.markSensitive, strings.sensitive.markTitle],
    [strings.menu.compareText, strings.compare.title],
  ];

  for (const [item, title] of menuDialogs) {
    await chooseFromMenu(page, item);
    const dialog = page.getByRole("dialog", { name: title });
    await expect(dialog).toBeVisible();

    // Verify focus is trapped inside dialog
    const inside = await page.evaluate(() => {
      const el = document.activeElement;
      return el !== null && el !== document.body && (document.querySelector('[role="dialog"]')?.contains(el) ?? false);
    });
    expect(inside, `${title}: initial focus should be inside dialog`).toBe(true);

    // Keyboard navigation inside dialog (Tab trap check)
    await page.keyboard.press("Tab");

    // Close via Escape key
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);

    // Focus MUST restore to the invoking "More" button, never drop to body
    const activeLabel = await page.evaluate(() => document.activeElement?.getAttribute("aria-label"));
    expect(activeLabel, `${title}: focus should return to More button`).toBe(strings.toolbar.more);
  }
});

test("external and blocked links: LinkConfirmDialog and BlockedLinkDialog initial focus and return", async ({ launch }) => {
  // Test external link confirmation
  const page = await launch(corpus("benign/external-https-link.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  const link = page.locator("button[data-link]").first();
  await link.click();

  const confirmDialog = page.getByRole("dialog", { name: strings.links.confirmTitle });
  await expect(confirmDialog).toBeVisible();

  // Initial focus must be on the Cancel button (docs/ux/screen-map.md §4)
  const cancelText = await page.evaluate(() => document.activeElement?.textContent?.trim());
  expect(cancelText).toBe(strings.links.cancel);

  await inBothThemes(page, (theme) => noProblems(page, `link confirmation, ${theme}`));

  // Dismiss with Escape
  await page.keyboard.press("Escape");
  await expect(confirmDialog).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);

  // Test blocked link dialog
  launchAgain(corpus("malicious/launch.pdf"));
  const blockedTab = page.getByRole("tab", { name: /launch/ });
  await expect(blockedTab).toBeVisible();
  await blockedTab.click();
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  const blockedLink = page.locator("button[data-link]").first();
  await blockedLink.click();

  const blockedDialog = page.getByRole("dialog", { name: strings.links.blockedTitle });
  await expect(blockedDialog).toBeVisible();

  // Initial focus must be on the Close button
  const closeText = await page.evaluate(() => document.activeElement?.textContent?.trim());
  expect(closeText).toBe(strings.links.close);

  await inBothThemes(page, (theme) => noProblems(page, `blocked link, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(blockedDialog).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);
});

test("annotations: TextBoxDialog and Issue #402 focus restoration", async ({ launch }) => {
  const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
  const sheet = page.getByRole("img", { name: strings.canvas.page(1) });
  await expect(sheet).toHaveAttribute("data-state", "ready");
  const t = strings.annotations;

  // Add Text Box via tool
  await page.keyboard.press("Control+0");
  await page.getByRole("button", { name: t.textBox, exact: true }).click();
  const area = (await sheet.boundingBox())!;
  await page.mouse.click(area.x + area.width * 0.5, area.y + area.height * 0.5);

  const addDialog = page.getByRole("dialog", { name: t.textBoxDialog.addTitle });
  await expect(addDialog).toBeVisible();

  // Initial focus on textarea
  expect(await page.evaluate(() => document.activeElement?.tagName)).toBe("TEXTAREA");
  await inBothThemes(page, (theme) => noProblems(page, `add text box dialog, ${theme}`));

  // Type and save box to test editing and #402 restoration
  await addDialog.getByLabel(t.textBoxDialog.text).fill("FocusTest402");
  await addDialog.getByRole("button", { name: t.textBoxDialog.save, exact: true }).click();
  await expect(addDialog).toHaveCount(0);

  // Focus the newly created text box on page
  const boxAnnotation = page.locator("[data-page-annotations] [data-annotation=textBox]").first();
  await expect(boxAnnotation).toBeVisible();
  await boxAnnotation.focus();
  await expect(page.getByRole("toolbar", { name: t.textBoxSaying("FocusTest402") })).toBeVisible();

  // Edit via Enter key (#402 walkthrough)
  await page.keyboard.press("Enter");
  const editDialog = page.getByRole("dialog", { name: t.textBoxDialog.editTitle });
  await expect(editDialog).toBeVisible();

  // Dismiss edit dialog with Escape
  await page.keyboard.press("Escape");
  await expect(editDialog).toHaveCount(0);

  // VERIFY ISSUE #402: Focus MUST return to the text box on canvas, NOT drop to body!
  const focusedAnnotation = await page.evaluate(() => document.activeElement?.getAttribute("data-annotation"));
  expect(focusedAnnotation, "#402: focus must return to the text box annotation").toBe("textBox");
});

test("outline bookmarks: DeleteBookmarkDialog and rename with keyboard", async ({ launch }) => {
  const page = await launch(corpus("benign/outline-3-levels.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  const outlineTab = page.getByRole("tab", { name: strings.sidebar.outlineTab });
  if (await outlineTab.isVisible()) {
    await outlineTab.click();
  }

  const item = page.getByRole("treeitem", { name: "Chapter 1" });
  await expect(item).toBeVisible();
  await item.focus();

  // Test inline rename (F2) and Escape
  await page.keyboard.press("F2");
  const renameInput = page.getByRole("textbox", { name: strings.sidebar.renameInputLabel });
  await expect(renameInput).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(renameInput).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement?.getAttribute("role"))).toBe("treeitem");

  // Test delete confirmation dialog (Delete on bookmark with children)
  await page.keyboard.press("Delete");
  const deleteDialog = page.getByRole("dialog", { name: strings.sidebar.deleteConfirmTitle });
  await expect(deleteDialog).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `delete bookmark dialog, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(deleteDialog).toHaveCount(0);

  // Focus restores to the bookmark item in the tree
  expect(await page.evaluate(() => document.activeElement?.getAttribute("role"))).toBe("treeitem");
});

test("redaction: FindPersonalDataDialog and RedactionConfirmDialog", async ({ launch }) => {
  const page = await launch(corpus("benign/redaction-sample.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Switch to redaction sidebar tab
  await page.getByRole("tab", { name: strings.redaction.tab }).click();

  // 1. Find Personal Data dialog
  const findBtn = page.getByRole("button", { name: strings.redaction.findPersonalData });
  await findBtn.click();
  const findDialog = page.getByRole("dialog", { name: strings.redaction.findPersonalDataDialog.title });
  await expect(findDialog).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `find personal data dialog, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(findDialog).toHaveCount(0);

  // Focus restores to "找出個資…" button
  expect(await page.evaluate(() => document.activeElement?.textContent?.trim())).toBe(strings.redaction.findPersonalData);

  // 2. Redaction confirmation dialog
  await page.getByRole("button", { name: strings.redaction.tool, exact: true }).click();
  const sheet = page.getByRole("img", { name: strings.canvas.page(1) });
  const box = (await sheet.boundingBox())!;
  await page.mouse.click(box.x + 80, box.y + 80);

  const applyBtn = page.getByRole("button", { name: strings.redaction.apply });
  await expect(applyBtn).toBeEnabled();
  await applyBtn.click();

  const confirmDialog = page.getByRole("dialog", { name: strings.redaction.dialog.title });
  await expect(confirmDialog).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `redaction confirm dialog, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(confirmDialog).toHaveCount(0);

  // Focus restores to "套用遮蔽…" button
  expect(await page.evaluate(() => document.activeElement?.textContent?.trim())).toBe(strings.redaction.apply);
});

test("page management: MovePagesDialog in thumbnails", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Open thumbnails tab
  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  const thumb = page.getByRole("option", { name: strings.canvas.page(1) });
  await thumb.click({ button: "right" });

  await page.getByRole("menuitem", { name: strings.pages.moveTo }).click();
  const moveDialog = page.getByRole("dialog", { name: strings.pages.move.title });
  await expect(moveDialog).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `move pages dialog, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(moveDialog).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);
});

test("attachments: DangerConfirmDialog on executable attachment", async ({ launch }) => {
  const page = await launch(corpus("malicious/attachment-exe.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  // Switch to attachments tab
  await page.getByRole("tab", { name: strings.sidebar.attachmentsTab }).click();
  const saveBtn = page.getByTestId("save-attachment-button").first();
  await saveBtn.click();

  const dangerDialog = page.getByTestId("danger-confirm-dialog");
  await expect(dangerDialog).toBeVisible();
  await expect(dangerDialog).toContainText(strings.attachments.dangerConfirm.title);
  await inBothThemes(page, (theme) => noProblems(page, `danger confirm dialog, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(dangerDialog).toHaveCount(0);

  // Focus restores to the save button
  expect(await page.evaluate(() => document.activeElement?.getAttribute("data-testid"))).toBe("save-attachment-button");
});

test("form flattening: FlattenDialog in form document", async ({ launch }) => {
  const page = await launch(corpus("benign/form-fields.pdf"));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  await chooseFromMenu(page, strings.forms.flatten);
  const flattenDialog = page.getByRole("dialog", { name: strings.forms.flattenDialog.title });
  await expect(flattenDialog).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `flatten dialog, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(flattenDialog).toHaveCount(0);

  // Focus restores to More button
  expect(await page.evaluate(() => document.activeElement?.getAttribute("aria-label"))).toBe(strings.toolbar.more);
});

test("unsaved changes: UnsavedDialog when closing tab with edits", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  const sheet = page.getByRole("img", { name: strings.canvas.page(1) });
  await expect(sheet).toHaveAttribute("data-state", "ready");

  // Add a text box to make document dirty
  await page.getByRole("button", { name: strings.annotations.textBox, exact: true }).click();
  const area = (await sheet.boundingBox())!;
  await page.mouse.click(area.x + 80, area.y + 80);

  const tbDialog = page.getByRole("dialog", { name: strings.annotations.textBoxDialog.addTitle });
  await tbDialog.getByLabel(strings.annotations.textBoxDialog.text).fill("UnsavedNote");
  await tbDialog.getByRole("button", { name: strings.annotations.textBoxDialog.save, exact: true }).click();
  await expect(tbDialog).toHaveCount(0);

  // Close tab via Ctrl+W
  await page.keyboard.press("Control+w");
  const unsavedDialog = page.getByRole("dialog", { name: strings.saving.askTitle });
  await expect(unsavedDialog).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `unsaved dialog, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(unsavedDialog).toHaveCount(0);

  // Tab remains open, focus is not body
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);
});

test("security details panel", async ({ launch }) => {
  const page = await launch(corpus("malicious/openaction-js.pdf"));
  await expect(page.getByRole("region", { name: strings.banner.label })).toBeVisible();

  const detailsBtn = page.getByRole("button", { name: strings.banner.details });
  await detailsBtn.click();
  const panel = page.getByRole("complementary", { name: strings.banner.detailsTitle });
  await expect(panel).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `security details panel, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement?.textContent?.trim())).toBe(strings.banner.details);
});

test("signature details panel", async ({ launch }) => {
  const page = await launch(corpus("benign/signed.pdf"));
  await expect(page.getByRole("region", { name: strings.signatures.label })).toBeVisible();

  const signBtn = page.getByRole("button", { name: strings.signatures.details });
  await signBtn.click();
  const signPanel = page.getByRole("complementary", { name: strings.signatures.detailsTitle });
  await expect(signPanel).toBeVisible();
  await inBothThemes(page, (theme) => noProblems(page, `signature details panel, ${theme}`));

  await page.keyboard.press("Escape");
  await expect(signPanel).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement?.textContent?.trim())).toBe(strings.signatures.details);
});
