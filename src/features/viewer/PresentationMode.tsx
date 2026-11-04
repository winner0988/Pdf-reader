// Full-screen presentation mode (B4-03, docs/ux/screen-map.md, section 7).

import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent } from "react";

import type { PageSize, Rotation } from "@/features/shell/model";
import { CSS_PX_PER_PT, displayedSize, rectToBox, renderScale } from "@/features/viewer/layout";
import { drawRaster, type PageRenderer } from "@/features/viewer/renderer";
import { linkHoverText } from "@/features/links/text";
import type { LinkSource } from "@/features/links/source";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, PageLink, Rotation as ContractRotation } from "@/ipc/generated/contract";
import { cn } from "@/lib/utils";

const CONTRACT_ROTATION: Record<Rotation, ContractRotation> = {
  0: "none",
  90: "cw90",
  180: "cw180",
  270: "cw270",
};

export type PresentationModeProps = {
  doc?: DocumentId;
  pages: PageSize[];
  rotation: Rotation;
  page: number;
  renderer: PageRenderer;
  links?: LinkSource;
  onPageChange: (page: number) => void;
  onClose: () => void;
  onLinkActivate: (link: PageLink) => void;
};

export function PresentationMode({
  doc,
  pages,
  rotation,
  page,
  renderer,
  links,
  onPageChange,
  onClose,
  onLinkActivate,
}: PresentationModeProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  // Window viewport measurement for fit-to-screen scale.
  const [viewport, setViewport] = useState({
    width: typeof window !== "undefined" ? window.innerWidth : 1920,
    height: typeof window !== "undefined" ? window.innerHeight : 1080,
  });

  useEffect(() => {
    const onResize = () => {
      setViewport({ width: window.innerWidth, height: window.innerHeight });
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // Cursor auto-hides after 3 seconds of inactivity.
  const [cursorHidden, setCursorHidden] = useState(false);
  const timerRef = useRef<number | undefined>(undefined);

  const resetCursorTimer = () => {
    setCursorHidden(false);
    if (timerRef.current !== undefined) {
      window.clearTimeout(timerRef.current);
    }
    timerRef.current = window.setTimeout(() => {
      setCursorHidden(true);
    }, 3000);
  };

  useEffect(() => {
    timerRef.current = window.setTimeout(() => {
      setCursorHidden(true);
    }, 3000);
    return () => {
      if (timerRef.current !== undefined) {
        window.clearTimeout(timerRef.current);
      }
    };
  }, []);

  // Focus the dialog layer on mount to capture keyboard events and trap assistive tech focus.
  useEffect(() => {
    containerRef.current?.focus();
  }, []);

  // Compute fitted dimensions for the current page.
  const pageIndex = Math.min(Math.max(page - 1, 0), pages.length - 1);
  const currentPageSize = pages[pageIndex] ?? { widthPt: 612, heightPt: 792 };
  const size = displayedSize(currentPageSize, rotation);
  const ptWidth = size.widthPt * CSS_PX_PER_PT;
  const ptHeight = size.heightPt * CSS_PX_PER_PT;
  const factor = Math.min(viewport.width / ptWidth, viewport.height / ptHeight);
  const cssWidth = Math.max(Math.round(ptWidth * factor), 1);
  const cssHeight = Math.max(Math.round(ptHeight * factor), 1);
  const scale = renderScale(factor, typeof window !== "undefined" ? window.devicePixelRatio : 1);

  // Render the current page onto the canvas.
  useEffect(() => {
    if (doc === undefined) return;
    let active = true;
    const contractRotation = CONTRACT_ROTATION[rotation];
    const job = renderer.render({ doc, pageIndex, scale, rotation: contractRotation });
    job.result
      .then((raster) => {
        if (active) {
          drawRaster(canvasRef.current, raster);
        }
      })
      .catch(() => {});

    // Prefetch next page if available.
    if (pageIndex + 1 < pages.length) {
      const nextSize = displayedSize(pages[pageIndex + 1]!, rotation);
      const nextPtW = nextSize.widthPt * CSS_PX_PER_PT;
      const nextPtH = nextSize.heightPt * CSS_PX_PER_PT;
      const nextFactor = Math.min(viewport.width / nextPtW, viewport.height / nextPtH);
      const nextScale = renderScale(nextFactor, typeof window !== "undefined" ? window.devicePixelRatio : 1);
      renderer.render({ doc, pageIndex: pageIndex + 1, scale: nextScale, rotation: contractRotation });
    }

    return () => {
      active = false;
      job.cancel();
    };
  }, [doc, pageIndex, scale, rotation, renderer, pages, viewport.width, viewport.height]);

  // Load links for the current page.
  const [pageLinks, setPageLinks] = useState<PageLink[]>([]);
  useEffect(() => {
    if (!links || doc === undefined) return;
    let current = true;
    links
      .links(doc, pageIndex)
      .then((items) => {
        if (current) setPageLinks(items);
      })
      .catch(() => {
        if (current) setPageLinks([]);
      });
    return () => {
      current = false;
    };
  }, [links, doc, pageIndex]);

  const displayedLinks = links && doc !== undefined ? pageLinks : [];

  // Page turning actions.
  const goNext = () => {
    if (page < pages.length) {
      onPageChange(page + 1);
    }
  };

  const goPrevious = () => {
    if (page > 1) {
      onPageChange(page - 1);
    }
  };

  const goFirst = () => {
    onPageChange(1);
  };

  const goLast = () => {
    onPageChange(pages.length);
  };

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    resetCursorTimer();
    // Stop propagation so background shell does not execute other shortcuts.
    event.stopPropagation();

    switch (event.key) {
      case "ArrowRight":
      case "ArrowDown":
      case "PageDown":
      case " ":
        event.preventDefault();
        goNext();
        break;
      case "ArrowLeft":
      case "ArrowUp":
      case "PageUp":
        event.preventDefault();
        goPrevious();
        break;
      case "Home":
        event.preventDefault();
        goFirst();
        break;
      case "End":
        event.preventDefault();
        goLast();
        break;
      case "Escape":
        event.preventDefault();
        onClose();
        break;
    }
  };

  const onClick = (event: ReactMouseEvent<HTMLDivElement>) => {
    // Only left click advances page.
    if (event.button === 0) {
      goNext();
    }
  };

  return (
    <div
      ref={containerRef}
      role="dialog"
      aria-modal="true"
      aria-label={strings.presentation.title}
      tabIndex={-1}
      className={cn(
        "fixed inset-0 z-40 flex items-center justify-center bg-black outline-none select-none",
        cursorHidden && "cursor-none",
      )}
      onPointerMove={resetCursorTimer}
      onKeyDown={onKeyDown}
      onClick={onClick}
    >
      <div className="sr-only" aria-live="polite" aria-atomic="true">
        {strings.presentation.pageAnnouncement(page, pages.length)}
      </div>
      <div
        className="relative flex items-center justify-center"
        style={{ width: cssWidth, height: cssHeight }}
      >
        <canvas
          ref={canvasRef}
          width={cssWidth}
          height={cssHeight}
          style={{ width: cssWidth, height: cssHeight }}
          className="block"
        />
        {displayedLinks.length > 0 && (
          <div
            data-page-links={page}
            className="pointer-events-none absolute inset-0"
          >
            {displayedLinks.map((link) => {
              const area = rectToBox(link.rect, currentPageSize, rotation, {
                width: cssWidth,
                height: cssHeight,
              });
              const text = linkHoverText(link.target);
              return (
                <button
                  key={link.id.index}
                  type="button"
                  aria-label={text}
                  data-link={link.target.kind}
                  className="pointer-events-auto absolute cursor-pointer rounded-[2px] outline-offset-1 focus-visible:outline-2 focus-visible:outline-primary"
                  style={{
                    left: area.left,
                    top: area.top,
                    width: area.width,
                    height: area.height,
                  }}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (link.target.kind === "page") {
                      onPageChange(link.target.pageIndex + 1);
                    } else {
                      onLinkActivate(link);
                    }
                  }}
                />
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
