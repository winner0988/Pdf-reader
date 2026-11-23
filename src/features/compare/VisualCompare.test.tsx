import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { CompareTabOption } from "@/features/compare/CompareDialog";
import { VisualCompareDialog } from "@/features/compare/VisualCompareDialog";
import type { RenderApi } from "@/features/viewer/renderer";
import { strings } from "@/i18n";
import type { RenderPageArgs } from "@/ipc/generated/contract";
import { expectAccessible } from "@/test/axe";

const t = strings.compare.visual;

function makeRasterBuffer(
  width: number,
  height: number,
  fill = 255,
  modRect?: { x: number; y: number; w: number; h: number; fill: number },
): ArrayBuffer {
  const headerBytes = 16;
  const pixelBytes = width * height * 4;
  const buffer = new ArrayBuffer(headerBytes + pixelBytes);
  const view = new DataView(buffer);
  view.setUint8(0, 0x50);
  view.setUint8(1, 0x44);
  view.setUint8(2, 0x46);
  view.setUint8(3, 0x52);
  view.setUint16(4, 1, true); // formatRgba8
  view.setUint16(6, 0, true);
  view.setUint32(8, width, true);
  view.setUint32(12, height, true);

  const pixels = new Uint8ClampedArray(buffer, headerBytes);
  pixels.fill(fill);
  for (let i = 3; i < pixels.length; i += 4) {
    pixels[i] = 255;
  }

  if (modRect) {
    for (let y = modRect.y; y < modRect.y + modRect.h; y++) {
      for (let x = modRect.x; x < modRect.x + modRect.w; x++) {
        const idx = (y * width + x) * 4;
        pixels[idx] = modRect.fill;
        pixels[idx + 1] = modRect.fill;
        pixels[idx + 2] = modRect.fill;
      }
    }
  }

  return buffer;
}

const mockTabs: CompareTabOption[] = [
  {
    tab: 2,
    doc: 20,
    displayName: "Doc-B.pdf",
    copyAllowed: true,
    pages: [{ widthPt: 500, heightPt: 500 }],
  },
];

describe("VisualCompareDialog (B5-14, #451)", () => {
  it("renders and displays 1 of 1 difference after starting compare", async () => {
    const user = userEvent.setup();
    const mockApi: RenderApi = {
      renderPage: vi.fn(async (args: RenderPageArgs) => {
        // Doc 10 is identical white; Doc 20 has a diff square
        if (args.doc === 20) {
          return makeRasterBuffer(1000, 1000, 255, { x: 50, y: 50, w: 20, h: 20, fill: 0 });
        }
        return makeRasterBuffer(1000, 1000, 255);
      }),
      cancel: vi.fn(async () => {}),
    };

    render(
      <VisualCompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        currentDisplayName="Doc-A.pdf"
        currentPages={[{ widthPt: 500, heightPt: 500 }]}
        otherTabs={mockTabs}
        renderApi={mockApi}
      />,
    );

    // Initial state shows start button
    const startBtn = screen.getByRole("button", { name: t.start });
    expect(startBtn).toBeInTheDocument();

    await user.click(startBtn);

    // Shows 1 of 1 diff
    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("第 1 處，共 1 處");
    });
  });

  it("shows no difference status for identical pages", async () => {
    const user = userEvent.setup();
    const mockApi: RenderApi = {
      renderPage: vi.fn(async () => makeRasterBuffer(1000, 1000, 255)),
      cancel: vi.fn(async () => {}),
    };

    render(
      <VisualCompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        currentDisplayName="Doc-A.pdf"
        currentPages={[{ widthPt: 500, heightPt: 500 }]}
        otherTabs={mockTabs}
        renderApi={mockApi}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(t.noDiff);
    });
  });

  it("indicates when a page only exists on one side", async () => {
    const user = userEvent.setup();
    const mockApi: RenderApi = {
      renderPage: vi.fn(async () => makeRasterBuffer(1000, 1000, 255)),
      cancel: vi.fn(async () => {}),
    };

    // Doc A has 2 pages, Doc B only has 1 page
    render(
      <VisualCompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        currentDisplayName="Doc-A.pdf"
        currentPages={[
          { widthPt: 500, heightPt: 500 },
          { widthPt: 500, heightPt: 500 },
        ]}
        otherTabs={mockTabs}
        renderApi={mockApi}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));

    // Go to next page (page 2)
    const nextBtn = await screen.findByRole("button", { name: t.nextPage });
    await user.click(nextBtn);

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(t.onlyInDoc("Doc-A.pdf"));
    });
  });

  it("displays size difference notice when page dimensions differ", async () => {
    const user = userEvent.setup();
    const mockApi: RenderApi = {
      renderPage: vi.fn(async () => makeRasterBuffer(1000, 1000, 255)),
      cancel: vi.fn(async () => {}),
    };

    render(
      <VisualCompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        currentDisplayName="Doc-A.pdf"
        currentPages={[{ widthPt: 600, heightPt: 500 }]} // 600 width vs Doc B 500 width
        otherTabs={mockTabs}
        renderApi={mockApi}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(t.sizeDiff);
    });
  });

  it("next diff jumps to subsequent page with differences", async () => {
    const user = userEvent.setup();
    const mockApi: RenderApi = {
      renderPage: vi.fn(async (args: RenderPageArgs) => {
        // Page 0 is identical
        if (args.pageIndex === 0) {
          return makeRasterBuffer(1000, 1000, 255);
        }
        // Page 1 has diff on Doc 20
        if (args.doc === 20 && args.pageIndex === 1) {
          return makeRasterBuffer(1000, 1000, 255, { x: 50, y: 50, w: 20, h: 20, fill: 0 });
        }
        return makeRasterBuffer(1000, 1000, 255);
      }),
      cancel: vi.fn(async () => {}),
    };

    render(
      <VisualCompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        currentDisplayName="Doc-A.pdf"
        currentPages={[
          { widthPt: 500, heightPt: 500 },
          { widthPt: 500, heightPt: 500 },
        ]}
        otherTabs={[
          {
            tab: 2,
            doc: 20,
            displayName: "Doc-B.pdf",
            copyAllowed: true,
            pages: [
              { widthPt: 500, heightPt: 500 },
              { widthPt: 500, heightPt: 500 },
            ],
          },
        ]}
        renderApi={mockApi}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));

    // Page 1 has no diff initially
    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(t.noDiff);
    });

    // Click next diff
    const nextDiffBtn = screen.getByRole("button", { name: t.nextDiff });
    await user.click(nextDiffBtn);

    // Moves to page 2 and shows diff
    await waitFor(() => {
      expect(screen.getByText(t.pageInfo(2, 2))).toBeInTheDocument();
      expect(screen.getByRole("status")).toHaveTextContent("第 1 處，共 1 處");
    });
  });

  it("cancels pending requests when closed", async () => {
    const user = userEvent.setup();
    let resolvePending: (buffer: ArrayBuffer) => void = () => {};
    const mockApi: RenderApi = {
      renderPage: vi.fn(
        () =>
          new Promise<ArrayBuffer>((res) => {
            resolvePending = res;
          }),
      ),
      cancel: vi.fn(async () => {}),
    };

    const onOpenChange = vi.fn();
    render(
      <VisualCompareDialog
        open={true}
        onOpenChange={onOpenChange}
        currentDoc={10}
        currentDisplayName="Doc-A.pdf"
        currentPages={[{ widthPt: 500, heightPt: 500 }]}
        otherTabs={mockTabs}
        renderApi={mockApi}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));

    // Click Close while pending
    await user.click(screen.getByRole("button", { name: t.close }));

    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(mockApi.cancel).toHaveBeenCalled();

    // Cleanup pending promise
    resolvePending(makeRasterBuffer(1000, 1000, 255));
  });

  it("passes axe accessibility checks", async () => {
    const user = userEvent.setup();
    const mockApi: RenderApi = {
      renderPage: vi.fn(async (args: RenderPageArgs) => {
        if (args.doc === 20) {
          return makeRasterBuffer(1000, 1000, 255, { x: 50, y: 50, w: 20, h: 20, fill: 0 });
        }
        return makeRasterBuffer(1000, 1000, 255);
      }),
      cancel: vi.fn(async () => {}),
    };

    render(
      <VisualCompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        currentDisplayName="Doc-A.pdf"
        currentPages={[{ widthPt: 500, heightPt: 500 }]}
        otherTabs={mockTabs}
        renderApi={mockApi}
      />,
    );

    // Initial state
    await expectAccessible(document.body);

    // Active compare state
    await user.click(screen.getByRole("button", { name: t.start }));
    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("第 1 處，共 1 處");
    });

    await expectAccessible(document.body);
  });
});
