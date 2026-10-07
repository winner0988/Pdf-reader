import { describe, expect, it } from "vitest";

import { bannerSummary, categoryOf, worstCategory } from "@/features/signatures/summary";
import { strings } from "@/i18n/zh-TW";
import type { SignatureInfo, SignatureReport } from "@/ipc/generated/contract";

const t = strings.signatures;

function signature(overrides: Partial<SignatureInfo> = {}): SignatureInfo {
  return {
    status: "valid",
    signerTrusted: false,
    reason: null,
    fieldName: "Signature1",
    signer: "Jane Public",
    claimedTime: null,
    certification: null,
    ...overrides,
  };
}

const trusted = signature({ signerTrusted: true });
const unconfirmed = signature();
const changed = signature({ status: "changedAfterSigning" });
const invalid = signature({ status: "invalid", signer: null });
const unverifiable = signature({ status: "unverifiable", reason: "unsupportedFormat", signer: null });

const report = (...signatures: SignatureInfo[]): SignatureReport => ({ signatures, truncated: false });

describe("what a signature is told as", () => {
  it("is valid only when its signer is trusted", () => {
    expect(categoryOf(trusted)).toBe("valid");
    expect(categoryOf(unconfirmed)).toBe("unconfirmed");
    expect(categoryOf(changed)).toBe("changed");
    expect(categoryOf(invalid)).toBe("invalid");
    expect(categoryOf(unverifiable)).toBe("unverifiable");
  });

  it("follows the worst of the signatures of a document", () => {
    expect(worstCategory(report(trusted))).toBe("valid");
    expect(worstCategory(report(trusted, unconfirmed))).toBe("unconfirmed");
    expect(worstCategory(report(trusted, unconfirmed, unverifiable))).toBe("unverifiable");
    expect(worstCategory(report(trusted, unverifiable, changed))).toBe("changed");
    expect(worstCategory(report(changed, invalid, trusted))).toBe("invalid");
  });
});

describe("the banner's sentence", () => {
  it("says what the one signature is", () => {
    expect(bannerSummary(report(trusted))).toBe(t.summaryOne.valid);
    expect(bannerSummary(report(unconfirmed))).toBe(t.summaryOne.unconfirmed);
    expect(bannerSummary(report(changed))).toBe(t.summaryOne.changed);
    expect(bannerSummary(report(invalid))).toBe(t.summaryOne.invalid);
    expect(bannerSummary(report(unverifiable))).toBe(t.summaryOne.unverifiable);
  });

  it("counts the kinds of several, worst first", () => {
    expect(bannerSummary(report(trusted, invalid, trusted, changed))).toBe(
      "此文件有 4 個數位簽章：1 個無效、1 個在簽署後有變更、2 個有效，簽署者受信任。",
    );
  });

  it("says when not all of the signature fields were looked at", () => {
    const text = bannerSummary({ signatures: [unconfirmed, unconfirmed], truncated: true });
    expect(text).toContain("2 個有效但無法確認簽署者");
    expect(text.endsWith(t.truncated(2))).toBe(true);
  });
});
