import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AttachmentInfo, DocumentId } from "@/ipc/generated/contract";
import { AttachmentsPanel } from "./AttachmentsPanel";
import { formatFileSize } from "./size";
import type { AttachmentsApi } from "./api";

const mockAttachments: AttachmentInfo[] = [
  {
    id: 1,
    name: "document.pdf",
    size: 2048,
    description: "Sample PDF document",
    source: { type: "document" },
    isDangerous: false,
    extension: ".pdf",
  },
  {
    id: 2,
    name: "payload.exe",
    size: 1048576,
    description: null,
    source: { type: "page", page: 0 },
    isDangerous: true,
    extension: ".exe",
  },
  {
    id: 3,
    name: "README",
    size: null,
    description: "No extension file",
    source: { type: "document" },
    isDangerous: true,
    extension: "",
  },
];

describe("formatFileSize", () => {
  it("formats bytes, KB and MB properly", () => {
    expect(formatFileSize(500)).toBe("500 B");
    expect(formatFileSize(1536)).toBe("1.5 KB");
    expect(formatFileSize(2097152)).toBe("2.0 MB");
  });
});

describe("AttachmentsPanel", () => {
  it("shows loading state initially", () => {
    const api: AttachmentsApi = {
      getAttachments: () => new Promise(() => {}),
      saveAttachment: vi.fn(),
    };

    render(<AttachmentsPanel doc={1 as DocumentId} api={api} />);
    expect(screen.getByText("正在讀取附件…")).toBeInTheDocument();
  });

  it("shows empty state when no attachments exist", async () => {
    const api: AttachmentsApi = {
      getAttachments: vi.fn().mockResolvedValue([]),
      saveAttachment: vi.fn(),
    };

    render(<AttachmentsPanel doc={1 as DocumentId} api={api} />);
    await waitFor(() => {
      expect(screen.getByText("這份文件沒有附件")).toBeInTheDocument();
    });
  });

  it("renders attachments list with true extensions and danger warnings", async () => {
    const api: AttachmentsApi = {
      getAttachments: vi.fn().mockResolvedValue(mockAttachments),
      saveAttachment: vi.fn(),
    };

    render(<AttachmentsPanel doc={1 as DocumentId} api={api} />);

    await waitFor(() => {
      expect(screen.getByText("共 3 個附件")).toBeInTheDocument();
    });

    expect(screen.getByText("document.pdf")).toBeInTheDocument();
    expect(screen.getByText(".pdf")).toBeInTheDocument();
    expect(screen.getByText("payload.exe")).toBeInTheDocument();
    expect(screen.getByText(".exe")).toBeInTheDocument();
    expect(screen.getByText("README")).toBeInTheDocument();
    expect(screen.getByText("無副檔名")).toBeInTheDocument();

    const dangerBadges = screen.getAllByTestId("attachment-dangerous");
    expect(dangerBadges).toHaveLength(2);
  });

  it("shows sensitive document warning banner when isSensitive is true", async () => {
    const api: AttachmentsApi = {
      getAttachments: vi.fn().mockResolvedValue(mockAttachments),
      saveAttachment: vi.fn(),
    };

    render(<AttachmentsPanel doc={1 as DocumentId} isSensitive={true} api={api} />);

    await waitFor(() => {
      expect(screen.getByTestId("sensitive-warning")).toBeInTheDocument();
    });
    expect(screen.getByText("存下來的檔案不在機敏保護內。")).toBeInTheDocument();
  });

  it("saves safe attachment directly without dangerous confirmation dialog", async () => {
    const saveAttachment = vi.fn().mockResolvedValue({ bytes: 2048, marked: true });
    const api: AttachmentsApi = {
      getAttachments: vi.fn().mockResolvedValue(mockAttachments),
      saveAttachment,
    };

    render(<AttachmentsPanel doc={1 as DocumentId} api={api} />);

    await waitFor(() => {
      expect(screen.getByText("document.pdf")).toBeInTheDocument();
    });

    const saveButtons = screen.getAllByTestId("save-attachment-button");
    fireEvent.click(saveButtons[0]!);

    await waitFor(() => {
      expect(saveAttachment).toHaveBeenCalledWith(1, 1);
    });
    expect(screen.queryByTestId("danger-confirm-dialog")).not.toBeInTheDocument();
  });

  it("requires confirmation before saving a dangerous attachment", async () => {
    const saveAttachment = vi.fn().mockResolvedValue({ bytes: 1048576, marked: true });
    const api: AttachmentsApi = {
      getAttachments: vi.fn().mockResolvedValue(mockAttachments),
      saveAttachment,
    };

    render(<AttachmentsPanel doc={1 as DocumentId} api={api} />);

    await waitFor(() => {
      expect(screen.getByText("payload.exe")).toBeInTheDocument();
    });

    const saveButtons = screen.getAllByTestId("save-attachment-button");
    // Click on payload.exe's save button
    fireEvent.click(saveButtons[1]!);

    // Danger confirm dialog must appear
    await waitFor(() => {
      expect(screen.getByTestId("danger-confirm-dialog")).toBeInTheDocument();
    });
    expect(screen.getByText("另存可執行檔案")).toBeInTheDocument();
    expect(
      screen.getByText("這是可以執行的檔案類型（.exe）。它來自 PDF，可能有害。只在你信任寄件者時才開啟它。", {
        exact: false,
      }),
    ).toBeInTheDocument();

    // Confirm save
    fireEvent.click(screen.getByTestId("danger-confirm-button"));

    await waitFor(() => {
      expect(saveAttachment).toHaveBeenCalledWith(1, 2);
    });
  });

  it("says so when the saved file could not be marked as from the internet", async () => {
    const saveAttachment = vi.fn().mockResolvedValue({ bytes: 2048, marked: false });
    const api: AttachmentsApi = {
      getAttachments: vi.fn().mockResolvedValue(mockAttachments),
      saveAttachment,
    };

    render(<AttachmentsPanel doc={1 as DocumentId} api={api} />);

    await waitFor(() => {
      expect(screen.getByText("document.pdf")).toBeInTheDocument();
    });

    const saveButtons = screen.getAllByTestId("save-attachment-button");
    fireEvent.click(saveButtons[0]!);

    await waitFor(() => {
      expect(
        screen.getByText("這個檔案沒有『來自網際網路』的標記，Windows 不會在開啟前額外檢查。"),
      ).toBeInTheDocument();
    });
  });
});
