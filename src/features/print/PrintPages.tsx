import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";

import { calculatePrintLayout, type PrintLayoutKind, type PrintPageOrder } from "@/features/print/layout";
import type { PrintPage } from "@/features/print/render";

type PrintPagesProps = {
  pages: readonly PrintPage[];
  layout?: PrintLayoutKind;
  order?: PrintPageOrder;
  border?: boolean;
  /** Printing is over (printed or cancelled in the system's dialog). */
  onDone: () => void;
};

/**
 * The pages being printed (MVP-17, B5-15), arranged into sheets according to the chosen
 * layout (1-up, multi-page N-up, or booklet). They are shown only to the printer
 * (index.css hides the app when printing, and these on screen); the system's
 * print dialog opens once every image is decoded, and they are removed when it closes.
 */
export function PrintPages({
  pages,
  layout = "1",
  order = "horizontal",
  border = false,
  onDone,
}: PrintPagesProps) {
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

  const sheets = calculatePrintLayout(pages, layout, order);

  return createPortal(
    <div ref={container} data-print-pages>
      {sheets.map((sheet) => (
        <div
          key={sheet.sheetIndex}
          data-print-sheet
          data-orientation={sheet.orientation}
          style={{
            gridTemplateRows: `repeat(${sheet.rows}, 1fr)`,
            gridTemplateColumns: `repeat(${sheet.cols}, 1fr)`,
          }}
        >
          {sheet.cells.map((page, cellIndex) => (
            <div
              key={cellIndex}
              data-print-cell
              data-has-page={page !== null ? "true" : undefined}
              data-border={border && page !== null ? "true" : undefined}
            >
              {page ? (
                <img
                  src={page.url}
                  alt=""
                  data-page={page.pageIndex + 1}
                  style={{ aspectRatio: `${page.size.widthPt} / ${page.size.heightPt}` }}
                />
              ) : null}
            </div>
          ))}
        </div>
      ))}
    </div>,
    document.body,
  );
}
