// Remote references (B3-08, ADR 0022) in the real app: the worker lists their web addresses,
// the main process describes one by its position, and nothing is opened unless the user confirms.
// The test never confirms: that would start this computer's browser.
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

const t = strings.banner;

test("a remote resource's web address can be looked at in the browser, after its full address is shown (B3-08)", async ({
  launch,
}) => {
  const page = await launch(corpus("malicious/remote-filespec.pdf"));
  const banner = page.getByRole("region", { name: t.label });
  await expect(banner).toContainText(strings.findings.remoteFileSpec.name);
  await banner.getByRole("button", { name: t.details }).click();
  const panel = page.getByRole("complementary", { name: t.detailsTitle });
  const addresses = panel.getByRole("region", { name: t.addressesTitle });
  await expect(addresses).toContainText("https://img.example.invalid/beacon.png");
  await expect(addresses.getByRole("button", { name: t.openAddress })).toHaveCount(1);

  await addresses.getByRole("button", { name: t.openAddress }).click();
  const dialog = page.getByRole("dialog", { name: strings.links.confirmTitle });
  // What the main process made of the worker's address: the site, and the whole URL.
  await expect(dialog.locator("[data-host]")).toHaveText("img.example.invalid");
  await expect(dialog.getByLabel(strings.links.confirmFullUrl)).toHaveText("https://img.example.invalid/beacon.png");
  await expect(dialog.getByRole("button", { name: strings.links.cancel })).toBeFocused();
  await dialog.getByRole("button", { name: strings.links.cancel }).click();
  await expect(dialog).toHaveCount(0);
});

test("where a form would be sent is not offered (B3-08)", async ({ launch }) => {
  const page = await launch(corpus("malicious/submitform.pdf"));
  const banner = page.getByRole("region", { name: t.label });
  await expect(banner).toContainText(strings.findings.submitForm.name);
  await banner.getByRole("button", { name: t.details }).click();
  const panel = page.getByRole("complementary", { name: t.detailsTitle });
  await expect(panel.locator('[data-kind="submitForm"]')).toBeVisible();
  await expect(panel.getByRole("region", { name: t.addressesTitle })).toHaveCount(0);
  await expect(panel.getByRole("button", { name: t.openAddress })).toHaveCount(0);
});
