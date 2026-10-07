// The signatures of a document in the real app (B2-14, ADR 0014): verified offline by the
// document's own worker, in its sandbox, with Windows' cryptography. The corpus's two signed files
// are signed with a self-signed test certificate: their signatures hold, and nobody vouches for
// the signer.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

const t = strings.signatures;
const SIGNER = "PDF Reader test corpus signer (NOT TRUSTED)";

test.describe("digital signatures", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-signatures-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("a signed document says its signature holds, but that nobody vouches for the signer", async ({ launch }) => {
    const page = await launch(corpus("benign/signed.pdf"));

    const banner = page.getByRole("region", { name: t.label });
    await expect(banner).toContainText(t.summaryOne.unconfirmed);
    await expect(banner).toHaveAttribute("data-signatures", "unconfirmed");
    // The page itself is shown as usual.
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

    await banner.getByRole("button", { name: t.details }).click();
    const panel = page.getByRole("complementary", { name: t.detailsTitle });
    await expect(panel).toContainText("Signature1");
    await expect(panel).toContainText(t.status.unconfirmed);
    await expect(panel).toContainText(SIGNER);
    await expect(panel).toContainText("2026-01-01 00:00:00 UTC");
    await expect(panel).toContainText(t.timeNote);
    await expect(panel).toContainText(t.detailsNote);
    // Closing the panel puts focus back on the banner's button.
    await panel.getByRole("button", { name: t.detailsClose }).click();
    await expect(panel).toHaveCount(0);
    await expect(banner.getByRole("button", { name: t.details })).toBeFocused();
    // The banner can be closed too.
    await banner.getByRole("button", { name: t.dismiss }).click();
    await expect(banner).toHaveCount(0);
  });

  test("a certifying signature says what it allows", async ({ launch }) => {
    const page = await launch(corpus("benign/signed-docmdp-p1.pdf"));

    const banner = page.getByRole("region", { name: t.label });
    await expect(banner).toContainText(t.summaryOne.unconfirmed);
    await banner.getByRole("button", { name: t.details }).click();
    const panel = page.getByRole("complementary", { name: t.detailsTitle });
    await expect(panel).toContainText(t.certification);
    await expect(panel).toContainText(t.certificationLevel.noChanges);
    // Nothing changed after it: no warning.
    await expect(panel).not.toContainText(t.certificationBroken);
  });

  test("a signature whose signed bytes were changed is invalid", async ({ launch }) => {
    const bytes = readFileSync(corpus("benign/signed.pdf"));
    // One letter of the page's text, inside what the signature covers.
    bytes[bytes.indexOf("Signed sample")] = "s".charCodeAt(0);
    const changed = path.join(folder, "changed.pdf");
    writeFileSync(changed, bytes);
    const page = await launch(changed);

    const banner = page.getByRole("region", { name: t.label });
    await expect(banner).toContainText(t.summaryOne.invalid);
    await expect(banner).toHaveAttribute("data-signatures", "invalid");
    await banner.getByRole("button", { name: t.details }).click();
    const panel = page.getByRole("complementary", { name: t.detailsTitle });
    await expect(panel).toContainText(t.status.invalid);
    // A signature that does not hold names nobody.
    await expect(panel).not.toContainText(SIGNER);
  });

  test("a document without signatures has no signature banner", async ({ launch }) => {
    const page = await launch(corpus("benign/multi-page-10.pdf"));

    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
    await expect(page.getByRole("region", { name: t.label })).toHaveCount(0);
  });
});
