import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createRef, useRef, type Ref } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { PageSize, Rotation, Zoom } from "@/features/shell/model";
import { DocumentView, type DocumentViewHandle } from "@/features/viewer/DocumentView";
import type { Highlights } from "@/features/viewer/highlights";
import { CSS_PX_PER_PT, PAGE_GAP_PX, PAGE_PADDING_PX } from "@/features/viewer/layout";
import type { PageRenderer, RenderJob } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { IpcError, RenderPageArgs } from "@/ipc/generated/contract";
import type { RasterImage } from "@/ipc/raster";

const LETTER = { widthPt: 612, heightPt: 792 };
const PITCH = 792 * CSS_PX_PER_PT + PAGE_GAP_PX; // one Letter page at 100% plus the gap

type Call = {
  args: Omit<RenderPageArgs, "request">;
  resolve: (raster: RasterImage) => void;
  reject: (error: IpcError) => void;
  cancel: ReturnType<typeof vi.fn>;
};

/** Records every render request; the test settles them. */
function fakeRenderer() {
  const calls: Call[] = [];
  const renderer: PageRenderer = {
    render(args) {
      let resolve: Call["resolve"] = () => {};
      let reject: Call["reject"] = () => {};
      const result = new Promise<RasterImage>((ok, fail) => {
        resolve = ok;
        reject = fail;
      });
      const cancel = vi.fn(() => reject({ code: "cancelled", message: "" }));
      calls.push({ args, resolve, reject, cancel });
      return { result, cancel } satisfies RenderJob;
    },
  };
  const active = () => calls.filter((call) => call.cancel.mock.calls.length === 0);
  return { renderer, calls, active };
}

const raster = (): RasterImage => ({ width: 2, height: 2, pixels: new Uint8ClampedArray(16) });

type HarnessProps = {
  pages: PageSize[];
  zoom?: Zoom;
  rotation?: Rotation;
  renderer?: PageRenderer;
  onPage?: (page: number) => void;
  onZoomStep?: (direction: 1 | -1) => void;
  highlights?: Highlights;
  view?: Ref<DocumentViewHandle>;
};

function Harness({ pages, zoom = 100, rotation = 0, renderer, onPage, onZoomStep, highlights, view }: HarnessProps) {
  const scroller = useRef<HTMLElement>(null);
  return (
    <main ref={scroller} data-testid="scroller" style={{ overflow: "auto" }}>
      <DocumentView
        ref={view}
        pages={pages}
        zoom={zoom}
        rotation={rotation}
        scrollContainer={scroller}
        doc={7}
        renderer={renderer}
        onCurrentPageChange={onPage}
        onZoomStep={onZoomStep}
        highlights={highlights}
        requestDelayMs={0}
      />
    </main>
  );
}

/** jsdom has no layout: give the scroller a size, and scroll it. */
function sizeScroller(height: number, width = 1000) {
  const element = screen.getByTestId("scroller");
  Object.defineProperty(element, "clientHeight", { configurable: true, value: height });
  Object.defineProperty(element, "clientWidth", { configurable: true, value: width });
  return element;
}

function scrollTo(element: HTMLElement, top: number) {
  act(() => {
    element.scrollTop = top;
    element.dispatchEvent(new Event("scroll"));
  });
}

const mountedPages = () =>
  screen.getAllByRole("img").map((page) => Number(page.getAttribute("aria-label")!.match(/\d+/)![0]));

let putImageData: ReturnType<typeof vi.fn>;

beforeEach(() => {
  // jsdom implements neither 2D canvas nor ImageData.
  putImageData = vi.fn();
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    putImageData,
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

describe("DocumentView", () => {
  it("mounts and renders only the pages near the viewport", () => {
    const { renderer, calls } = fakeRenderer();
    const { rerender } = render(<Harness pages={Array(1000).fill(LETTER)} renderer={renderer} />);
    sizeScroller(800);
    rerender(<Harness pages={Array(1000).fill(LETTER)} renderer={renderer} />);
    scrollTo(screen.getByTestId("scroller"), 0);

    expect(mountedPages()).toEqual([1, 2, 3]);
    expect(calls.map((call) => call.args.pageIndex)).toEqual([0, 1, 2]);
    expect(calls[0]!.args).toEqual({ doc: 7, pageIndex: 0, scale: 1.333, rotation: "none" });
    // The scroll content still has the height of all 1000 pages.
    const content = screen.getByTestId("scroller").firstElementChild as HTMLElement;
    expect(parseFloat(content.style.height)).toBeGreaterThan(PITCH * 999);
  });

  it("cancels pages that scroll out of range and requests the new ones", () => {
    const { renderer, calls, active } = fakeRenderer();
    render(<Harness pages={Array(100).fill(LETTER)} renderer={renderer} />);
    const scroller = sizeScroller(800);
    scrollTo(scroller, 0);
    const first = calls.slice(0, 3);

    // Page 51 (index 50) fills the viewport; two pages on each side stay mounted.
    scrollTo(scroller, PAGE_PADDING_PX + PITCH * 50);
    expect(mountedPages()).toEqual([49, 50, 51, 52, 53]);
    for (const call of first) expect(call.cancel).toHaveBeenCalled();
    expect(active().map((call) => call.args.pageIndex).sort((a, b) => a - b)).toEqual([48, 49, 50, 51, 52]);
  });

  it("draws rendered pages at their native resolution", async () => {
    const { renderer, calls } = fakeRenderer();
    render(<Harness pages={[LETTER]} renderer={renderer} />);
    await act(async () => calls[0]!.resolve(raster()));

    const page = screen.getByRole("img", { name: "第 1 頁" });
    expect(page).toHaveAttribute("data-state", "ready");
    const canvas = page.querySelector("canvas")!;
    expect([canvas.width, canvas.height]).toEqual([2, 2]);
    expect(putImageData).toHaveBeenCalledTimes(1);
  });

  it("shows a failed page with a retry button, without affecting the others", async () => {
    const { renderer, calls } = fakeRenderer();
    const user = userEvent.setup();
    render(<Harness pages={[LETTER, LETTER]} renderer={renderer} />);
    await act(async () => {
      calls[0]!.reject({ code: "workerCrashed", message: "" });
      calls[1]!.resolve(raster());
    });

    const failed = screen.getByRole("img", { name: "第 1 頁" });
    expect(failed).toHaveTextContent(strings.canvas.pageRenderFailed);
    expect(screen.getByRole("img", { name: "第 2 頁" })).toHaveAttribute("data-state", "ready");

    await user.click(screen.getByRole("button", { name: strings.error.retry }));
    expect(calls).toHaveLength(3);
    expect(calls[2]!.args.pageIndex).toBe(0);
    await act(async () => calls[2]!.resolve(raster()));
    expect(failed).toHaveAttribute("data-state", "ready");
  });

  it("re-renders at the new resolution once the zoom settles", () => {
    vi.useFakeTimers();
    try {
      const { renderer, calls } = fakeRenderer();
      const { rerender } = render(<Harness pages={[LETTER]} renderer={renderer} />);
      act(() => vi.advanceTimersByTime(1)); // the first scale is kept from here on
      rerender(<Harness pages={[LETTER]} zoom={200} renderer={renderer} />);
      // While the zoom is changing, the old request is dropped and nothing new is asked for.
      expect(calls).toHaveLength(1);
      expect(calls[0]!.cancel).toHaveBeenCalled();
      rerender(<Harness pages={[LETTER]} zoom={300} renderer={renderer} />);
      act(() => vi.advanceTimersByTime(100));
      rerender(<Harness pages={[LETTER]} zoom={200} renderer={renderer} />);
      act(() => vi.advanceTimersByTime(100));
      expect(calls).toHaveLength(1);
      act(() => vi.advanceTimersByTime(100));
      expect(calls).toHaveLength(2);
      expect(calls[1]!.args.scale).toBe(2.667);
    } finally {
      vi.useRealTimers();
    }
  });

  it("reports the page being read and scrolls to a page on request", () => {
    const onPage = vi.fn();
    const view = createRef<DocumentViewHandle>();
    render(<Harness pages={Array(20).fill(LETTER)} onPage={onPage} view={view} />);
    const scroller = sizeScroller(800);

    // Browsers fire `scroll` after scrollTop changes; jsdom does not.
    act(() => {
      view.current!.scrollToPage(10);
      scroller.dispatchEvent(new Event("scroll"));
    });
    expect(onPage).toHaveBeenLastCalledWith(10);
    expect(mountedPages()).toContain(10);
  });

  it("without a renderer (demo data) pages stay placeholders", () => {
    render(<Harness pages={[LETTER]} />);
    expect(screen.getByRole("img", { name: "第 1 頁" })).toHaveAttribute("data-state", "loading");
  });

  it("does not render pages that only flash by", () => {
    vi.useFakeTimers();
    try {
      const { renderer, calls } = fakeRenderer();
      function Delayed() {
        const scroller = useRef<HTMLElement>(null);
        return (
          <main ref={scroller} data-testid="scroller">
            <DocumentView
              pages={Array(100).fill(LETTER)}
              zoom={100}
              rotation={0}
              scrollContainer={scroller}
              doc={7}
              renderer={renderer}
            />
          </main>
        );
      }
      render(<Delayed />);
      const element = sizeScroller(800);
      scrollTo(element, 0);
      // Fly past pages 20-40 faster than the request delay.
      for (let page = 20; page <= 40; page += 5) scrollTo(element, PAGE_PADDING_PX + PITCH * page);
      expect(calls).toHaveLength(0);
      act(() => vi.advanceTimersByTime(100)); // REQUEST_DELAY_MS
      // Only the pages where the scroll stopped are rendered, the one on screen first.
      expect(calls.map((call) => call.args.pageIndex)).toEqual([40]);
      act(() => vi.advanceTimersByTime(150)); // NEARBY_EXTRA_DELAY_MS
      expect(calls.map((call) => call.args.pageIndex).sort((a, b) => a - b)).toEqual([38, 39, 40, 41, 42]);
    } finally {
      vi.useRealTimers();
    }
  });

  describe("zoom and rotation (MVP-08)", () => {
    const pages: PageSize[] = Array(100).fill(LETTER);

    it("Ctrl+wheel zooms in and out instead of the WebView, plain wheel scrolls", () => {
      const onZoomStep = vi.fn();
      render(<Harness pages={pages} onZoomStep={onZoomStep} />);
      const scroller = sizeScroller(800);
      const wheel = (init: WheelEventInit) => {
        const event = new WheelEvent("wheel", { cancelable: true, ...init });
        scroller.dispatchEvent(event);
        return event;
      };

      expect(wheel({ ctrlKey: true, deltaY: -100 }).defaultPrevented).toBe(true);
      expect(onZoomStep).toHaveBeenLastCalledWith(1);
      wheel({ ctrlKey: true, deltaY: 100 });
      expect(onZoomStep).toHaveBeenLastCalledWith(-1);
      // Small (touchpad) deltas add up to one step.
      wheel({ ctrlKey: true, deltaY: -30 });
      expect(onZoomStep).toHaveBeenCalledTimes(2);
      wheel({ ctrlKey: true, deltaY: -30 });
      expect(onZoomStep).toHaveBeenCalledTimes(3);

      expect(wheel({ deltaY: 100 }).defaultPrevented).toBe(false);
      expect(onZoomStep).toHaveBeenCalledTimes(3);
    });

    it("stays on page 50 when zooming to 400% and back", () => {
      const onPage = vi.fn();
      const { rerender } = render(<Harness pages={pages} onPage={onPage} />);
      const scroller = sizeScroller(800, 1000);
      const start = PAGE_PADDING_PX + PITCH * 49 + 200; // 200 px into page 50
      scrollTo(scroller, start);
      expect(onPage).toHaveBeenLastCalledWith(50);

      rerender(<Harness pages={pages} zoom={400} onPage={onPage} />);
      act(() => scroller.dispatchEvent(new Event("scroll")));
      expect(onPage).toHaveBeenLastCalledWith(50);
      expect(scroller.scrollLeft).toBeGreaterThan(0); // the page is wider than the window now
      const page = screen.getByRole("img", { name: "第 50 頁" });
      expect(parseFloat(page.style.left)).toBe(PAGE_PADDING_PX);

      rerender(<Harness pages={pages} zoom={100} onPage={onPage} />);
      act(() => scroller.dispatchEvent(new Event("scroll")));
      expect(onPage).toHaveBeenLastCalledWith(50);
      expect(scroller.scrollTop).toBeCloseTo(start, 0);
      expect(scroller.scrollLeft).toBe(0);
    });

    it("the page being read keeps its bitmap while zooming (it never unmounts)", async () => {
      const { renderer, calls } = fakeRenderer();
      const { rerender } = render(<Harness pages={pages} renderer={renderer} />);
      const scroller = sizeScroller(800, 1000);
      scrollTo(scroller, PAGE_PADDING_PX + PITCH * 49 + 200);
      const page50 = screen.getByRole("img", { name: "第 50 頁" });
      const request = calls.find((call) => call.args.pageIndex === 49)!;
      await act(async () => request.resolve(raster()));
      expect(page50).toHaveAttribute("data-state", "ready");

      for (const zoom of [110, 125, 150, 200, 400]) {
        rerender(<Harness pages={pages} zoom={zoom} renderer={renderer} />);
        expect(screen.getByRole("img", { name: "第 50 頁" })).toBe(page50);
      }
      expect(page50).toHaveAttribute("data-state", "ready");
    });

    it("two zoom steps before the scroll event arrives still keep the page", () => {
      const onPage = vi.fn();
      const { rerender } = render(<Harness pages={pages} onPage={onPage} />);
      const scroller = sizeScroller(800, 1000);
      scrollTo(scroller, PAGE_PADDING_PX + PITCH * 49 + 200);

      rerender(<Harness pages={pages} zoom={200} onPage={onPage} />);
      rerender(<Harness pages={pages} zoom={400} onPage={onPage} />);
      act(() => scroller.dispatchEvent(new Event("scroll")));
      expect(onPage).toHaveBeenLastCalledWith(50);
    });

    it("keeps the page when rotating, with swapped page sizes", () => {
      const onPage = vi.fn();
      const { rerender } = render(<Harness pages={pages} onPage={onPage} />);
      const scroller = sizeScroller(800, 1200);
      scrollTo(scroller, PAGE_PADDING_PX + PITCH * 29 + 100);
      expect(onPage).toHaveBeenLastCalledWith(30);

      rerender(<Harness pages={pages} rotation={90} onPage={onPage} />);
      act(() => scroller.dispatchEvent(new Event("scroll")));
      expect(onPage).toHaveBeenLastCalledWith(30);
      const page = screen.getByRole("img", { name: "第 30 頁" });
      expect(parseFloat(page.style.width)).toBeCloseTo(792 * CSS_PX_PER_PT);
      expect(parseFloat(page.style.height)).toBeCloseTo(612 * CSS_PX_PER_PT);
      // The scroll range follows: 100 landscape pages are shorter than 100 portrait ones.
      const content = scroller.firstElementChild as HTMLElement;
      expect(parseFloat(content.style.height)).toBeLessThan(PITCH * 100);
    });
  });

  describe("search highlights (MVP-10)", () => {
    const pages = Array(20).fill(LETTER);
    const line = (x: number, y: number) => ({
      ul: { x, y },
      ur: { x: x + 72, y },
      ll: { x, y: y + 12 },
      lr: { x: x + 72, y: y + 12 },
    });
    const highlights: Highlights = {
      hits: [
        { pageIndex: 0, quads: [line(72, 72)] },
        // Two lines: one hit, one outline around each line.
        { pageIndex: 1, quads: [line(72, 144), line(72, 156)] },
        { pageIndex: 12, quads: [line(300, 600)] },
      ],
      current: 1,
    };
    const polygons = (page: number, selector = "polygon") =>
      Array.from(screen.getByRole("img", { name: `第 ${page} 頁` }).querySelectorAll(selector));

    it("marks every hit and outlines the current one", () => {
      render(<Harness pages={pages} highlights={highlights} />);
      sizeScroller(800);
      scrollTo(screen.getByTestId("scroller"), 0);

      expect(polygons(1)).toHaveLength(1);
      expect(polygons(1, "[data-current]")).toHaveLength(0);
      expect(polygons(2, "polygon:not([data-current])")).toHaveLength(2);
      expect(polygons(2, "[data-current]").map((polygon) => polygon.getAttribute("points"))).toEqual([
        "96,192 192,192 192,208 96,208",
        "96,208 192,208 192,224 96,224",
      ]);
      expect(polygons(3)).toHaveLength(0);
    });

    it("keeps the marks on the text when zoomed and rotated", () => {
      const { rerender } = render(<Harness pages={pages} highlights={highlights} />);
      sizeScroller(800, 1200);
      scrollTo(screen.getByTestId("scroller"), 0);

      rerender(<Harness pages={pages} zoom={200} highlights={highlights} />);
      expect(polygons(1)[0]!.getAttribute("points")).toBe("192,192 384,192 384,224 192,224");
      rerender(<Harness pages={pages} zoom={200} rotation={90} highlights={highlights} />);
      const page = screen.getByRole("img", { name: "第 1 頁" });
      expect(page.querySelector("svg")!.getAttribute("viewBox")).toBe(`0 0 ${page.style.width.replace("px", "")} ${page.style.height.replace("px", "")}`);
      expect(polygons(1)[0]!.getAttribute("points")).toBe("1920,192 1920,384 1888,384 1888,192");
    });

    it("scrolls a hit to a third of the way down the view", () => {
      const view = createRef<DocumentViewHandle>();
      render(<Harness pages={pages} highlights={highlights} view={view} />);
      const scroller = sizeScroller(900);
      scrollTo(scroller, 0);

      act(() => view.current!.revealHit(highlights.hits[2]!));
      expect(scroller.scrollTop).toBeCloseTo(PAGE_PADDING_PX + PITCH * 12 + 600 * CSS_PX_PER_PT - 300);
      act(() => scroller.dispatchEvent(new Event("scroll")));
      expect(polygons(13)).toHaveLength(1);
    });
  });
});
