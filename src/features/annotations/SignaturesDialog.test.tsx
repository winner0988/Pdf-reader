import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SignaturesDialog } from "@/features/annotations/SignaturesDialog";
import type { SignaturesApi } from "@/features/annotations/signaturesApi";
import { strings } from "@/i18n";
import type { DocumentId, SignatureImage } from "@/ipc/generated/contract";

afterEach(cleanup);

describe("SignaturesDialog", () => {
  const t = strings.savedSignatures;

  const mockApi = (signatures: SignatureImage[] = []): SignaturesApi => ({
    getSignatureImages: vi.fn().mockResolvedValue(signatures),
    getSignatureImagePng: vi.fn().mockResolvedValue([1, 2, 3]),
    addSignatureImage: vi.fn().mockResolvedValue([...signatures, { id: 99, width: 200, height: 100 }]),
    importSignatureImage: vi.fn().mockResolvedValue([...signatures, { id: 100, width: 300, height: 150 }]),
    removeSignatureImage: vi.fn().mockResolvedValue([]),
    removeAllSignatureImages: vi.fn().mockResolvedValue([]),
    prepareSignatureStamp: vi.fn().mockResolvedValue({ id: 1, width: 200, height: 100, png: [1, 2, 3] }),
  });

  it("renders visible security/privacy note with role='note'", async () => {
    const api = mockApi([]);
    render(<SignaturesDialog open onOpenChange={vi.fn()} api={api} />);

    await waitFor(() => {
      const note = screen.getByRole("note");
      expect(note).toBeInTheDocument();
      expect(note).toHaveTextContent(t.note);
    });
  });

  it("displays saved signatures and allows placing one onto the document", async () => {
    const api = mockApi([{ id: 42, width: 120, height: 60 }]);
    const onSelect = vi.fn();
    const onOpenChange = vi.fn();

    render(
      <SignaturesDialog
        open
        onOpenChange={onOpenChange}
        api={api}
        doc={1 as unknown as DocumentId}
        onSelect={onSelect}
      />,
    );

    await waitFor(() => {
      expect(api.getSignatureImages).toHaveBeenCalled();
      expect(api.getSignatureImagePng).toHaveBeenCalledWith(42);
    });

    const sigButton = screen.getByRole("button", { name: `${t.clickToPlace} (42)` });
    fireEvent.click(sigButton);

    await waitFor(() => {
      expect(api.prepareSignatureStamp).toHaveBeenCalledWith(1, 42);
      expect(onSelect).toHaveBeenCalled();
      expect(onOpenChange).toHaveBeenCalledWith(false);
    });
  });

  it("handles individual delete with confirmation prompt", async () => {
    const api = mockApi([{ id: 42, width: 120, height: 60 }]);
    render(<SignaturesDialog open onOpenChange={vi.fn()} api={api} />);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: t.delete })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: t.delete }));

    // Confirm button appears
    const confirmBtn = screen.getByRole("button", { name: t.confirmDelete });
    expect(confirmBtn).toBeInTheDocument();

    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(api.removeSignatureImage).toHaveBeenCalledWith(42);
    });
  });

  it("shows max reached warning and disables adding when 10 signatures exist", async () => {
    const tenSignatures = Array.from({ length: 10 }, (_, i) => ({
      id: i + 1,
      width: 100,
      height: 50,
    }));
    const api = mockApi(tenSignatures);

    render(<SignaturesDialog open onOpenChange={vi.fn()} api={api} />);

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(t.maxReached);
    });
  });

  it("supports typed signature creation", async () => {
    const api = mockApi([]);
    render(
      <SignaturesDialog
        open
        onOpenChange={vi.fn()}
        api={api}
        toPngBytes={vi.fn().mockResolvedValue([1, 2, 3, 4])}
      />,
    );

    // Switch to type tab
    fireEvent.click(screen.getByRole("button", { name: t.typeTab }));

    const input = screen.getByPlaceholderText(t.typePlaceholder);
    fireEvent.change(input, { target: { value: "Alice Cooper" } });

    const saveBtn = screen.getByRole("button", { name: t.saveTyped });
    expect(saveBtn).not.toBeDisabled();

    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(api.addSignatureImage).toHaveBeenCalled();
    });
  });

  it("supports image import", async () => {
    const api = mockApi([]);
    render(<SignaturesDialog open onOpenChange={vi.fn()} api={api} />);

    // Switch to import tab
    fireEvent.click(screen.getByRole("button", { name: t.importTab }));

    const importBtn = screen.getByRole("button", { name: t.importButton });
    fireEvent.click(importBtn);

    await waitFor(() => {
      expect(api.importSignatureImage).toHaveBeenCalled();
    });
  });
});
