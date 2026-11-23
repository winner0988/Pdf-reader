import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { SignaturesApi } from "@/features/annotations/signaturesApi";
import { SignaturesSection } from "@/features/settings/SignaturesSection";
import { strings } from "@/i18n";
import type { SignatureImage } from "@/ipc/generated/contract";

afterEach(cleanup);

describe("SignaturesSection", () => {
  const t = strings.settings.signatures;

  const mockApi = (signatures: SignatureImage[] = []): SignaturesApi => ({
    getSignatureImages: vi.fn().mockResolvedValue(signatures),
    getSignatureImagePng: vi.fn().mockResolvedValue([]),
    addSignatureImage: vi.fn().mockResolvedValue([]),
    importSignatureImage: vi.fn().mockResolvedValue([]),
    removeSignatureImage: vi.fn().mockResolvedValue([]),
    removeAllSignatureImages: vi.fn().mockResolvedValue([]),
    prepareSignatureStamp: vi.fn().mockResolvedValue({ id: 1, width: 100, height: 50, png: [] }),
  });

  it("displays none saved message when count is 0 and remove-all is disabled", async () => {
    const api = mockApi([]);
    render(<SignaturesSection api={api} />);

    await waitFor(() => {
      expect(screen.getByText(t.noneSaved)).toBeInTheDocument();
    });

    const button = screen.getByRole("button", { name: t.removeAll });
    expect(button).toBeDisabled();
  });

  it("displays saved count when signatures exist and allows remove all with confirmation", async () => {
    const api = mockApi([{ id: 1, width: 100, height: 50 }]);
    render(<SignaturesSection api={api} />);

    await waitFor(() => {
      expect(screen.getByText(t.savedCount(1))).toBeInTheDocument();
    });

    const button = screen.getByRole("button", { name: t.removeAll });
    expect(button).not.toBeDisabled();

    fireEvent.click(button);

    // Confirmation UI appears
    expect(screen.getByText(t.confirmRemoveAllTitle)).toBeInTheDocument();
    expect(screen.getByText(t.confirmRemoveAllText)).toBeInTheDocument();

    const confirmButton = screen.getByRole("button", { name: t.confirmRemoveAll });
    fireEvent.click(confirmButton);

    await waitFor(() => {
      expect(api.removeAllSignatureImages).toHaveBeenCalledTimes(1);
      expect(screen.getByText(t.allRemoved)).toBeInTheDocument();
    });
  });

  it("allows cancelling remove-all confirmation", async () => {
    const api = mockApi([{ id: 1, width: 100, height: 50 }]);
    render(<SignaturesSection api={api} />);

    await waitFor(() => {
      expect(screen.getByText(t.savedCount(1))).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: t.removeAll }));
    expect(screen.getByText(t.confirmRemoveAllTitle)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: t.cancel }));
    expect(screen.queryByText(t.confirmRemoveAllTitle)).not.toBeInTheDocument();
    expect(api.removeAllSignatureImages).not.toHaveBeenCalled();
  });
});
