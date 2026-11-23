import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { PropertiesApi } from "@/features/properties/api";
import { DocumentPropertiesDialog } from "@/features/properties/DocumentPropertiesDialog";
import {
  formatPageSizes,
  formatPdfDate,
  formatSinglePageSize,
} from "@/features/properties/format";
import { strings } from "@/i18n";
import type { DocumentFonts, DocumentProperties } from "@/ipc/generated/contract";
import { expectAccessible } from "@/test/axe";

const t = strings.properties;

const mockProperties: DocumentProperties = {
  displayName: "test-doc.pdf",
  fileSize: 204800,
  pageCount: 3,
  pageSizes: [
    { widthPt: 595.28, heightPt: 841.89 },
    { widthPt: 595.28, heightPt: 841.89 },
    { widthPt: 595.28, heightPt: 841.89 },
  ],
  format: "PDF 1.7",
  encryptionMethod: null,
  title: "Test Document Title",
  author: "Author Name",
  subject: "Subject Line",
  keywords: "keyword1, keyword2",
  creator: "Application Creator",
  producer: "PDF Producer Library",
  creationDate: {
    year: 2024,
    month: 1,
    day: 15,
    hour: 9,
    minute: 30,
    second: 0,
    tzOffsetMinutes: 480,
  },
  modDate: {
    year: 2024,
    month: 2,
    day: 20,
    hour: 14,
    minute: 45,
    second: 0,
    tzOffsetMinutes: -300,
  },
  hasXmp: true,
  signaturesCount: 0,
  signaturesTruncated: false,
  attachmentCount: 2,
  security: {
    scanComplete: true,
    findings: [],
    remoteAddresses: [],
  },
  permissions: {
    print: true,
    printHighQuality: true,
    copy: true,
    modify: false,
    assemble: false,
    annotate: true,
    fillForms: true,
  },
  sensitive: false,
  encrypted: false,
};

const mockFonts: DocumentFonts = {
  fonts: [
    {
      name: "Helvetica",
      subtype: "Type 1",
      embedded: false,
    },
    {
      name: "LiberationSerif",
      subtype: "TrueType",
      embedded: true,
    },
  ],
  complete: true,
};

describe("properties formatting helpers (B5-04)", () => {
  it("formats dates according to PDF spec and offsets", () => {
    expect(formatPdfDate(null)).toBe("-");
    expect(
      formatPdfDate({ year: 2024, month: 5, day: 1, hour: 12, minute: 0, second: 0, tzOffsetMinutes: null }),
    ).toBe("2024-05-01 12:00:00");
    expect(
      formatPdfDate({ year: 2024, month: 5, day: 1, hour: 12, minute: 0, second: 0, tzOffsetMinutes: 0 }),
    ).toBe("2024-05-01 12:00:00 UTC");
    expect(
      formatPdfDate({ year: 2024, month: 5, day: 1, hour: 12, minute: 0, second: 0, tzOffsetMinutes: 480 }),
    ).toBe("2024-05-01 12:00:00 UTC+08:00");
    expect(
      formatPdfDate({ year: 2024, month: 5, day: 1, hour: 12, minute: 0, second: 0, tzOffsetMinutes: -300 }),
    ).toBe("2024-05-01 12:00:00 UTC-05:00");
  });

  it("formats single page sizes and multiple page sizes", () => {
    const a4 = { widthPt: 595.28, heightPt: 841.89 };
    const letter = { widthPt: 612, heightPt: 792 };
    expect(formatSinglePageSize(a4)).toContain("210 × 297 mm");
    expect(formatPageSizes([], t)).toBe("-");
    expect(formatPageSizes([a4, a4], t)).toContain("210 × 297 mm");
    expect(formatPageSizes([a4, letter], t)).toContain("多種尺寸");
  });
});

describe("DocumentPropertiesDialog (B5-04)", () => {
  function renderDialog(options: {
    properties?: DocumentProperties;
    fonts?: DocumentFonts;
    propertiesRejection?: unknown;
    fontsRejection?: unknown;
    onPrivacyExport?: () => void;
  } = {}) {
    const cancelFn = vi.fn(() => Promise.resolve());
    const api: PropertiesApi = {
      getDocumentProperties: vi.fn(() =>
        "propertiesRejection" in options
          ? Promise.reject(options.propertiesRejection)
          : Promise.resolve(options.properties ?? mockProperties),
      ),
      getDocumentFonts: vi.fn(() =>
        "fontsRejection" in options
          ? Promise.reject(options.fontsRejection)
          : Promise.resolve(options.fonts ?? mockFonts),
      ),
      cancel: cancelFn,
    };

    const onOpenChange = vi.fn();
    const onPrivacyExport = options.onPrivacyExport ?? vi.fn();

    const utils = render(
      <DocumentPropertiesDialog
        open
        onOpenChange={onOpenChange}
        doc={101}
        api={api}
        onPrivacyExport={onPrivacyExport}
      />,
    );

    return {
      api,
      cancelFn,
      onOpenChange,
      onPrivacyExport,
      user: userEvent.setup(),
      ...utils,
    };
  }

  it("renders basic document information in the first tab", async () => {
    renderDialog();

    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();

    expect(await screen.findByText("Test Document Title")).toBeInTheDocument();
    expect(screen.getByText("test-doc.pdf")).toBeInTheDocument();
    expect(screen.getByText("Author Name")).toBeInTheDocument();
    expect(screen.getByText("Subject Line")).toBeInTheDocument();
    expect(screen.getByText("keyword1, keyword2")).toBeInTheDocument();
    expect(screen.getByText("Application Creator")).toBeInTheDocument();
    expect(screen.getByText("PDF Producer Library")).toBeInTheDocument();
    expect(screen.getByText("PDF 1.7")).toBeInTheDocument();
    expect(screen.getByText(t.pagesCount(3))).toBeInTheDocument();
    expect(screen.getByText(t.xmpPresent)).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument(); // attachment count
  });

  it("triggers privacy export shortcut from basic info tab", async () => {
    const onPrivacyExport = vi.fn();
    const { user, onOpenChange } = renderDialog({ onPrivacyExport });

    const btn = await screen.findByRole("button", { name: t.privacyExportShortcut });
    await user.click(btn);

    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onPrivacyExport).toHaveBeenCalled();
  });

  it("does not show privacy export shortcut if document is encrypted", async () => {
    renderDialog({
      properties: {
        ...mockProperties,
        encrypted: true,
        encryptionMethod: "Standard V4 (AES-128)",
      },
    });

    await screen.findByText("Test Document Title");
    expect(screen.queryByRole("button", { name: t.privacyExportShortcut })).not.toBeInTheDocument();
  });

  it("switches to security tab and displays security details and permissions", async () => {
    const { user } = renderDialog({
      properties: {
        ...mockProperties,
        encrypted: true,
        encryptionMethod: "Standard V5 (AES-256)",
        sensitive: true,
        signaturesCount: 2,
        security: {
          scanComplete: true,
          findings: [
            { kind: "javaScript", count: 1 },
          ],
          remoteAddresses: [],
        },
      },
    });

    await screen.findByText("Test Document Title");

    await user.click(screen.getByRole("tab", { name: t.tabSecurity }));

    expect(screen.getByText("Standard V5 (AES-256)")).toBeInTheDocument();
    expect(screen.getByText(t.signaturesCount(2))).toBeInTheDocument();
    expect(screen.getByText(t.sensitiveMarked)).toBeInTheDocument();

    expect(screen.getByText(t.permPrint)).toBeInTheDocument();
    expect(screen.getByText(t.permCopy)).toBeInTheDocument();
    expect(screen.getByText(t.permModify)).toBeInTheDocument();
    expect(screen.getByText(t.permAssemble)).toBeInTheDocument();

    expect(screen.getByText(strings.findings.javaScript.name)).toBeInTheDocument();
  });

  it("switches to fonts tab and renders discovered fonts and incomplete warning", async () => {
    const { user } = renderDialog({
      fonts: {
        fonts: [
          {
            name: "TestFont-Regular",
            subtype: "TrueType",
            embedded: true,
          },
        ],
        complete: false,
      },
    });

    await screen.findByText("Test Document Title");

    await user.click(screen.getByRole("tab", { name: t.tabFonts }));

    expect(await screen.findByText("TestFont-Regular")).toBeInTheDocument();
    expect(screen.getByText("TrueType")).toBeInTheDocument();
    expect(screen.getByText(t.embeddedYes)).toBeInTheDocument();
    expect(screen.getByText(t.fontsTruncated)).toBeInTheDocument();
  });

  it("cancels font scan on unmount", async () => {
    const fontsPromise = new Promise<DocumentFonts>(() => {});
    const cancelFn = vi.fn(() => Promise.resolve());
    const api: PropertiesApi = {
      getDocumentProperties: vi.fn(() => Promise.resolve(mockProperties)),
      getDocumentFonts: vi.fn(() => fontsPromise),
      cancel: cancelFn,
    };

    const { unmount } = render(
      <DocumentPropertiesDialog
        open
        onOpenChange={vi.fn()}
        doc={101}
        api={api}
      />,
    );

    unmount();
    expect(cancelFn).toHaveBeenCalled();
  });

  it("shows the general reason, not the raw error, when the properties cannot be read (#634)", async () => {
    renderDialog({ propertiesRejection: new Error("Failed to load properties") });

    expect(await screen.findByText(t.loadPropertiesError)).toBeInTheDocument();
    expect(screen.getByText(strings.error.messages.internal)).toBeInTheDocument();
    expect(screen.queryByText(/Failed to load properties/)).toBeNull();
  });

  it("shows why the properties cannot be read instead of [object Object] (#634)", async () => {
    renderDialog({
      propertiesRejection: { code: "workerTimeout", message: "worker timed out" },
    });

    expect(await screen.findByText(t.loadPropertiesError)).toBeInTheDocument();
    expect(screen.getByText(strings.error.messages.workerTimeout)).toBeInTheDocument();
    expect(screen.queryByText(/object Object/)).toBeNull();
    expect(screen.queryByText(/worker timed out/)).toBeNull();
  });

  it("shows why the font list cannot be loaded (#634)", async () => {
    const { user } = renderDialog({
      fontsRejection: { code: "workerCrashed", message: "x" },
    });

    await screen.findByText("Test Document Title");

    await user.click(screen.getByRole("tab", { name: t.tabFonts }));

    expect(await screen.findByText(t.loadFontsError)).toBeInTheDocument();
    expect(screen.getByText(strings.error.messages.workerCrashed)).toBeInTheDocument();
  });

  it("renders ultra-long strings without breaking", async () => {
    const longString = "A".repeat(2000);
    renderDialog({
      properties: {
        ...mockProperties,
        displayName: `${longString}.pdf`,
        title: longString,
        author: longString,
      },
    });

    expect(await screen.findByText(`${longString}.pdf`)).toBeInTheDocument();
    expect(screen.getAllByText(longString).length).toBeGreaterThan(0);
  });

  it("passes axe accessibility checks across all tabs", async () => {
    const { user } = renderDialog();
    await screen.findByText("Test Document Title");

    // General tab
    await expectAccessible(document.body);

    // Security tab
    await user.click(screen.getByRole("tab", { name: t.tabSecurity }));
    await expectAccessible(document.body);

    // Fonts tab
    await user.click(screen.getByRole("tab", { name: t.tabFonts }));
    await expectAccessible(document.body);
  });
});
