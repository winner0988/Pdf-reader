import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { SavingApi } from "@/features/saving/api";
import { SanitizeDialog } from "@/features/saving/SanitizeDialog";
import type { SignatureView } from "@/features/signatures/useSignatures";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import type { SanitizeReport, SanitizeResult } from "@/ipc/generated/contract";

const t = strings.sanitize;

const emptyReport: SanitizeReport = {
  actions: 0,
  links: 0,
  attachments: 0,
  formScripts: 0,
  xfa: 0,
  richMedia: 0,
  hiddenData: 0,
  annotations: 0,
};

function fakeSavingApi(sanitizeCopy?: SavingApi["sanitizeCopy"]) {
  return {
    save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false, redaction: null })),
    saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve(null)),
    closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    privacyExport: vi.fn<SavingApi["privacyExport"]>(() => Promise.resolve(true)),
    encryptCopy: vi.fn<SavingApi["encryptCopy"]>(() => Promise.resolve(true)),
    compressImages: vi.fn<SavingApi["compressImages"]>(() => Promise.resolve(null)),
    sanitizeCopy: vi.fn<NonNullable<SavingApi["sanitizeCopy"]>>(
      sanitizeCopy ?? (() => Promise.resolve({ bytes: 100, report: emptyReport })),
    ),
  } satisfies SavingApi;
}

function renderShell(
  api?: SavingApi,
  docOverrides: Partial<typeof demoDocument> = {},
  signatures?: SignatureView,
) {
  const state: ShellState = {
    kind: "open",
    document: { ...demoDocument, doc: 42, ...docOverrides },
  };
  const user = userEvent.setup();
  render(
    <ReaderShell
      state={state}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      savingApi={api}
      signatures={signatures}
    />,
  );
  return { user };
}

async function moreMenuItem(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  return screen.findByRole("menuitem", { name: new RegExp(strings.menu.sanitizeCopy) });
}

describe("SanitizeDialog (B3-15)", () => {
  it("shows default values and Kept list", async () => {
    const onSanitize = vi.fn(() => Promise.resolve(null));
    render(
      <SanitizeDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={42}
        sensitive={true}
        onSanitize={onSanitize}
      />,
    );

    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: t.actions })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: t.keepWebLinks })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: t.attachments })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: t.formScripts })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: t.xfa })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: t.richMedia })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: t.hiddenData })).toBeChecked();

    expect(screen.getByRole("radio", { name: t.annotations.keep })).toBeChecked();
    expect(screen.getByRole("radio", { name: t.annotations.remove })).not.toBeChecked();
    expect(screen.getByRole("radio", { name: t.annotations.flatten })).not.toBeChecked();

    expect(screen.getByText(t.sensitiveNote)).toBeInTheDocument();
    for (const kept of t.kept) {
      expect(screen.getByText(kept)).toBeInTheDocument();
    }

    // The sensitive mark is document information: kept when hidden data is not cleared.
    await userEvent.click(screen.getByRole("checkbox", { name: t.hiddenData }));
    expect(screen.getByText(t.sensitiveKeptNote)).toBeInTheDocument();
    expect(screen.queryByText(t.sensitiveNote)).not.toBeInTheDocument();
  });

  it("submits configured options to onSanitize", async () => {
    const user = userEvent.setup();
    const onSanitize = vi.fn<NonNullable<SavingApi["sanitizeCopy"]>>(() =>
      Promise.resolve<SanitizeResult | null>({
        bytes: 500,
        report: { ...emptyReport, actions: 3, links: 1 },
      }),
    );
    render(
      <SanitizeDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={42}
        sensitive={false}
        onSanitize={onSanitize}
      />,
    );

    // Uncheck keepWebLinks and switch annotation mode to flatten
    await user.click(screen.getByRole("checkbox", { name: t.keepWebLinks }));
    await user.click(screen.getByRole("radio", { name: t.annotations.flatten }));

    await user.click(screen.getByRole("button", { name: t.start }));

    expect(onSanitize).toHaveBeenCalledWith({
      doc: 42,
      options: {
        actions: true,
        attachments: true,
        formScripts: true,
        xfa: true,
        richMedia: true,
        hiddenData: true,
        keepWebLinks: false,
        annotations: "flatten",
      },
    });

    await waitFor(() => {
      expect(screen.getByRole("dialog", { name: t.doneTitle })).toBeInTheDocument();
    });
    expect(screen.getByText(t.report.actions(3))).toBeInTheDocument();
    expect(screen.getByText(t.report.links(1))).toBeInTheDocument();
  });

  it("displays no items removed when report counts are zero", async () => {
    const user = userEvent.setup();
    const onSanitize = vi.fn<NonNullable<SavingApi["sanitizeCopy"]>>(() =>
      Promise.resolve<SanitizeResult | null>({ bytes: 200, report: emptyReport }),
    );
    render(
      <SanitizeDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={42}
        sensitive={false}
        onSanitize={onSanitize}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));

    await waitFor(() => {
      expect(screen.getByRole("dialog", { name: t.doneTitle })).toBeInTheDocument();
    });
    expect(screen.getByText(t.noItemsRemoved)).toBeInTheDocument();
  });

  it("displays error message on failure", async () => {
    const user = userEvent.setup();
    const onSanitize = vi.fn<NonNullable<SavingApi["sanitizeCopy"]>>(() => Promise.reject(new Error("failed")));
    render(
      <SanitizeDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={42}
        sensitive={false}
        onSanitize={onSanitize}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent(t.failed);
    });
  });

  it("disables menu item when document is encrypted", async () => {
    const api = fakeSavingApi();
    const { user } = renderShell(api, { encrypted: true });
    const item = await moreMenuItem(user);

    expect(item).toHaveAttribute("data-disabled");
    expect(item).toHaveTextContent(t.encrypted);
  });

  it("disables menu item when document is signed", async () => {
    const api = fakeSavingApi();
    const signatures: SignatureView = {
      status: "ready",
      report: {
        signatures: [
          {
            status: "valid",
            signerTrusted: true,
            reason: null,
            fieldName: "Sig1",
            signer: "Signer",
            claimedTime: null,
            certification: null,
          },
        ],
        truncated: false,
      },
    };
    const { user } = renderShell(api, {}, signatures);
    const item = await moreMenuItem(user);

    expect(item).toHaveAttribute("data-disabled");
    expect(item).toHaveTextContent(t.signatures);
  });
});
