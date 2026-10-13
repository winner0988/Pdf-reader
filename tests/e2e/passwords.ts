// What the specs about remembered passwords (B2-26) share: the corpus sample whose password they
// remember, and how they leave nothing of it behind. The app keeps remembered passwords in the
// Credential Manager of whoever runs the tests, so a test must never leave its own entry there.
import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, quit, type LaunchOptions } from "./app";

export const SAMPLE = corpus("benign/encrypted-aes256.pdf");

const t = strings.password;
export const asking = (page: Page) => page.getByRole("heading", { name: t.title });
export const notice = (page: Page) => page.getByRole("region", { name: t.recalled.label });

type Launch = (file?: string, options?: LaunchOptions) => Promise<Page>;

/**
 * Takes away the entry of the sample, if the app has one: it opens the file, and if it opens
 * without a question, forgets the password from the notice, as a user would. What a test left
 * behind, or never made, is the same to it.
 */
async function forgetSample(launch: Launch, data: string) {
  const page = await launch(SAMPLE, { dataDir: data });
  await expect(asking(page).or(notice(page))).toBeVisible();
  if (await notice(page).isVisible()) {
    await notice(page).getByRole("button", { name: t.recalled.forget }).click();
    await expect(notice(page)).toHaveCount(0);
  }
  await quit(page);
}

/**
 * For a test's `finally`: ends the app the test was using (a second start would only hand its file
 * to the one that runs, which is single-instance), then takes the sample's password away. A
 * problem here is said, not thrown: it must not hide why the test failed.
 */
export async function leaveNothing(launch: Launch, page: Page, data: string) {
  await quit(page).catch(() => {});
  try {
    await forgetSample(launch, data);
  } catch (error) {
    console.warn(
      `the sample's password may still be in the Credential Manager (forget it in the app): ${String(error).split("\n")[0]}`,
    );
  }
}
