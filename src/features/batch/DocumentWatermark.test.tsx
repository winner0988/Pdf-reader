import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DocumentWatermarkDialog } from "./DocumentWatermarkDialog";
import { strings } from "@/i18n/zh-TW";
import type { PageSize, StampImageId, StampImageInfo } from "@/ipc/generated/contract";
import type { PageRenderer, RenderJob } from "@/features/viewer/renderer";
import type { AnnotationsApi } from "@/features/annotations/source";

const t = strings.documentWatermark;
const pages: PageSize[] = [
  { widthPt: 595, heightPt: 842 },
  { widthPt: 595, heightPt: 842 },
  { widthPt: 595, heightPt: 842 },
];

function mockRenderer(): PageRenderer {
  return {
    render: vi.fn((): RenderJob => ({
      result: Promise.resolve({
        width: 100,
        height: 100,
        pixels: new Uint8ClampedArray(100 * 100 * 4),
      }),
      cancel: vi.fn(),
    })),
  };
}

describe("DocumentWatermarkDialog", () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
      save: vi.fn(),
      restore: vi.fn(),
      scale: vi.fn(),
      translate: vi.fn(),
      rotate: vi.fn(),
      measureText: vi.fn().mockReturnValue({ width: 100 }),
      fillRect: vi.fn(),
      strokeRect: vi.fn(),
      fillText: vi.fn(),
      putImageData: vi.fn(),
    } as unknown as CanvasRenderingContext2D);
    vi.stubGlobal(
      "ImageData",
      class {
        data: Uint8ClampedArray;
        width: number;
        height: number;
        constructor(data: Uint8ClampedArray, width: number, height: number) {
          this.data = data;
          this.width = width;
          this.height = height;
        }
      },
    );
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("renders preview and enables confirm button by default for text watermark", async () => {
    const applyEdit = vi.fn().mockResolvedValue(undefined);
    const onFinished = vi.fn();
    const onOpenChange = vi.fn();
    const renderer = mockRenderer();

    render(
      <DocumentWatermarkDialog
        open={true}
        onOpenChange={onOpenChange}
        doc={1}
        pages={pages}
        renderer={renderer}
        applyEdit={applyEdit}
        onFinished={onFinished}
      />,
    );

    expect(screen.getByRole("heading", { name: t.title })).toBeInTheDocument();
    expect(screen.getByText(t.description)).toBeInTheDocument();
    expect(screen.getByText(t.preview)).toBeInTheDocument();

    const submitBtn = screen.getByRole("button", { name: t.apply });
    expect(submitBtn).toBeEnabled();

    await waitFor(() => {
      expect(renderer.render).toHaveBeenCalledWith({
        doc: 1,
        pageIndex: 0,
        scale: 0.5,
        rotation: "none",
      });
    });
  });

  it("handles page range selection and validates range input", async () => {
    const user = userEvent.setup();
    const applyEdit = vi.fn().mockResolvedValue(undefined);

    render(
      <DocumentWatermarkDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={1}
        pages={pages}
        applyEdit={applyEdit}
        onFinished={vi.fn()}
      />,
    );

    const rangeRadio = screen.getByRole("radio", { name: t.pagesRange });
    await user.click(rangeRadio);

    const submitBtn = screen.getByRole("button", { name: t.apply });
    // Empty range text -> disabled
    expect(submitBtn).toBeDisabled();

    // Type invalid range (out of bounds)
    const rangeInput = screen.getByPlaceholderText(t.pagesRangePlaceholder);
    fireEvent.change(rangeInput, { target: { value: "1-10" } });
    expect(screen.getByText(t.invalidPages)).toBeInTheDocument();
    expect(submitBtn).toBeDisabled();

    // Type valid range
    fireEvent.change(rangeInput, { target: { value: "2-3" } });
    expect(screen.queryByText(t.invalidPages)).not.toBeInTheDocument();
    expect(submitBtn).toBeEnabled();

    // Apply with valid range
    await user.click(submitBtn);
    expect(applyEdit).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "addWatermark",
        pages: [1, 2],
      }),
    );
  });

  it("disables apply button for picture watermark until image is selected", async () => {
    const user = userEvent.setup();
    const applyEdit = vi.fn().mockResolvedValue(undefined);
    const mockImage: StampImageInfo = { image: 10 as StampImageId, width: 200, height: 100 };
    const annotationsApi: Pick<AnnotationsApi, "pickStampImage"> = {
      pickStampImage: vi.fn().mockResolvedValue(mockImage),
    };

    render(
      <DocumentWatermarkDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={1}
        pages={pages}
        annotationsApi={annotationsApi}
        applyEdit={applyEdit}
        onFinished={vi.fn()}
      />,
    );

    // Switch to picture watermark
    const pictureRadio = screen.getByRole("radio", { name: strings.batch.watermarkPicture });
    await user.click(pictureRadio);

    const submitBtn = screen.getByRole("button", { name: t.apply });
    expect(screen.getByText(t.noPictureChosen)).toBeInTheDocument();
    expect(submitBtn).toBeDisabled();

    // Pick image
    const chooseBtn = screen.getByRole("button", { name: t.choosePicture });
    await user.click(chooseBtn);

    await waitFor(() => {
      expect(annotationsApi.pickStampImage).toHaveBeenCalledWith(1);
      expect(screen.getByText(t.pictureSize(200, 100))).toBeInTheDocument();
      expect(submitBtn).toBeEnabled();
    });

    await user.click(submitBtn);
    expect(applyEdit).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "addWatermark",
        image: 10,
        watermark: expect.objectContaining({
          content: {
            kind: "picture",
            widthPt: 150,
          },
        }),
      }),
    );
  });

  it("displays error message if applyEdit rejects with signed error", async () => {
    const user = userEvent.setup();
    const applyEdit = vi.fn().mockRejectedValue({ code: "signed" });

    render(
      <DocumentWatermarkDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={1}
        pages={pages}
        applyEdit={applyEdit}
        onFinished={vi.fn()}
      />,
    );

    const submitBtn = screen.getByRole("button", { name: t.apply });
    await user.click(submitBtn);

    await waitFor(() => {
      expect(screen.getByText(t.signedDocument)).toBeInTheDocument();
    });
  });

  it("does not call renderer when sensitive is true", () => {
    const renderer = mockRenderer();

    render(
      <DocumentWatermarkDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={1}
        pages={pages}
        sensitive={true}
        renderer={renderer}
        applyEdit={vi.fn()}
        onFinished={vi.fn()}
      />,
    );

    expect(renderer.render).not.toHaveBeenCalled();
  });

  it("closes dialog when cancel button is clicked", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();

    render(
      <DocumentWatermarkDialog
        open={true}
        onOpenChange={onOpenChange}
        doc={1}
        pages={pages}
        applyEdit={vi.fn()}
        onFinished={vi.fn()}
      />,
    );

    const cancelBtn = screen.getByRole("button", { name: t.cancel });
    await user.click(cancelBtn);

    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
