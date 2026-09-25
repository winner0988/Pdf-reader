import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";

import type { PrintPage } from "@/features/print/render";

type PrintPagesProps = {
  pages: readonly PrintPage[];
  /** Printing is over (printed or cancelled in the system's dialog). */
  onDone: () => void;
};

/**
 * The pages being printed (MVP-17), one image per page in document order. They are shown only
 * to the printer (index.css hides the app when printing, and these on screen); the system's
 * print dialog opens once every image is decoded, and they are removed when it closes.
 */
export function PrintPages({ pages, onDone }: PrintPagesProps) {
  const container = useRef<HTMLDivElement>(null);
  const done = useRef(onDone);
  useEffect(() => {
    done.current = onDone;
  });

  useEffect(() => {
    let finished = false;
    const finish = () => {
      if (finished) return;
      finished = true;
      done.current();
    };
    window.addEventListener("afterprint", finish);
    const images = Array.from(container.current?.querySelectorAll("img") ?? []);
    // A page that is not decoded yet would print blank.
    void Promise.all(images.map((image) => image.decode?.().catch(() => {}))).then(() => {
      if (finished) return;
      window.print();
    });
    return () => {
      finished = true;
      window.removeEventListener("afterprint", finish);
    };
  }, []);

  return createPortal(
    <div ref={container} data-print-pages>
      {pages.map((page) => (
        <img
          key={page.pageIndex}
          src={page.url}
          alt=""
          data-page={page.pageIndex + 1}
          style={{ aspectRatio: `${page.size.widthPt} / ${page.size.heightPt}` }}
        />
      ))}
    </div>,
    document.body,
  );
}
