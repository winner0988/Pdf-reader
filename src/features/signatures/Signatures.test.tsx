import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import type { ShellDocument } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { SignatureView } from "@/features/signatures/useSignatures";
import { strings } from "@/i18n/zh-TW";
import type { SignatureInfo } from "@/ipc/generated/contract";

const t = strings.signatures;

function signature(overrides: Partial<SignatureInfo> = {}): SignatureInfo {
  return {
    status: "valid",
    signerTrusted: false,
    reason: null,
    fieldName: "Signature1",
    signer: "PDF Reader test corpus signer (NOT TRUSTED)",
    claimedTime: "2026-01-01 00:00:00 UTC",
    certification: null,
    ...overrides,
  };
}

function renderShell(view: SignatureView, document: Partial<ShellDocument> = {}) {
  const state = {
    kind: "open" as const,
    document: { ...demoDocument, doc: 5, session: 1, ...document },
  };
  const rendered = render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} signatures={view} />);
  return { ...rendered, user: userEvent.setup() };
}

const ready = (...signatures: SignatureInfo[]): SignatureView => ({
  status: "ready",
  report: { signatures, truncated: false },
});
const banner = () => screen.queryByRole("region", { name: t.label });

describe("the signature banner", () => {
  it("is not shown for a document without signatures", () => {
    renderShell({ status: "none" });
    expect(banner()).toBeNull();
  });

  it("says that the signature holds but nobody vouches for the signer", () => {
    renderShell(ready(signature()));
    expect(banner()).toHaveTextContent(t.summaryOne.unconfirmed);
    expect(banner()).toHaveAttribute("data-signatures", "unconfirmed");
  });

  it("is green only when the signer is trusted", () => {
    renderShell(ready(signature({ signerTrusted: true })));
    expect(banner()).toHaveAttribute("data-signatures", "valid");
    expect(banner()).toHaveTextContent(t.summaryOne.valid);
  });

  it("is red for a signature that does not hold", () => {
    renderShell(ready(signature({ status: "invalid", signer: null })));
    expect(banner()).toHaveAttribute("data-signatures", "invalid");
    expect(banner()).toHaveTextContent(t.summaryOne.invalid);
  });

  it("can be closed", async () => {
    const { user } = renderShell(ready(signature()));
    await user.click(within(banner()!).getByRole("button", { name: t.dismiss }));
    expect(banner()).toBeNull();
  });
});

describe("the signatures' details", () => {
  it("name the signer and the time the signer claims, and say that revocation is not checked", async () => {
    const { user } = renderShell(ready(signature()));
    await user.click(within(banner()!).getByRole("button", { name: t.details }));
    const panel = screen.getByRole("complementary", { name: t.detailsTitle });
    expect(panel).toHaveTextContent("Signature1");
    expect(panel).toHaveTextContent(t.status.unconfirmed);
    expect(panel).toHaveTextContent("PDF Reader test corpus signer (NOT TRUSTED)");
    expect(panel).toHaveTextContent("2026-01-01 00:00:00 UTC");
    expect(panel).toHaveTextContent(t.timeNote);
    expect(panel).toHaveTextContent(t.detailsNote);
    // Closing returns to the banner's button.
    await user.click(within(panel).getByRole("button", { name: t.detailsClose }));
    expect(screen.queryByRole("complementary", { name: t.detailsTitle })).toBeNull();
    expect(screen.getByRole("button", { name: t.details })).toHaveFocus();
  });

  it("say why a signature could not be verified, and name no signer", async () => {
    const { user } = renderShell(
      ready(signature({ status: "unverifiable", reason: "unsupportedAlgorithm", signer: null, fieldName: null })),
    );
    await user.click(within(banner()!).getByRole("button", { name: t.details }));
    const panel = screen.getByRole("complementary", { name: t.detailsTitle });
    expect(panel).toHaveTextContent(t.unnamed(1));
    expect(panel).toHaveTextContent(t.status.unverifiable);
    expect(panel).toHaveTextContent(t.reason.unsupportedAlgorithm);
    expect(within(panel).queryByText(t.signer)).toBeNull();
  });

  it("warn when a certifying signature allows no change and the document changed", async () => {
    const { user } = renderShell(ready(signature({ status: "changedAfterSigning", certification: "noChanges" })));
    await user.click(within(banner()!).getByRole("button", { name: t.details }));
    const panel = screen.getByRole("complementary", { name: t.detailsTitle });
    expect(panel).toHaveTextContent(t.certificationBroken);
    expect(panel).toHaveTextContent(t.certificationLevel.noChanges);
  });

  it("say when only some of the signature fields were looked at", async () => {
    const { user } = renderShell({ status: "ready", report: { signatures: [signature()], truncated: true } });
    await user.click(within(banner()!).getByRole("button", { name: t.details }));
    expect(screen.getByRole("complementary", { name: t.detailsTitle })).toHaveTextContent(t.truncated(1));
  });

  it("take turns with the blocked content's details", async () => {
    const { user } = renderShell(ready(signature()), {
      findings: [{ kind: "javaScript", count: 2 }],
      scanComplete: true,
    });
    await user.click(within(banner()!).getByRole("button", { name: t.details }));
    expect(screen.getByRole("complementary", { name: t.detailsTitle })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: strings.banner.details }));
    expect(screen.queryByRole("complementary", { name: t.detailsTitle })).toBeNull();
    expect(screen.getByRole("complementary", { name: strings.banner.detailsTitle })).toBeInTheDocument();
    await user.click(within(banner()!).getByRole("button", { name: t.details }));
    expect(screen.queryByRole("complementary", { name: strings.banner.detailsTitle })).toBeNull();
    expect(screen.getByRole("complementary", { name: t.detailsTitle })).toBeInTheDocument();
  });
});
