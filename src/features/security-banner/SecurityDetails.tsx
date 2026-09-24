import { TriangleAlert, X } from "lucide-react";
import { useEffect, useRef } from "react";

import { Button } from "@/components/ui/button";
import { LEAKY_KINDS, orderedFindings } from "@/features/security-banner/summary";
import { strings } from "@/i18n/zh-TW";
import type { SecurityFinding } from "@/ipc/generated/contract";

const t = strings.banner;

type SecurityDetailsProps = {
  findings: SecurityFinding[];
  scanComplete: boolean;
  onClose: () => void;
};

/**
 * The blocked-content details panel (docs/ux/screen-map.md, section 5): one row per kind with
 * its name, what it is and how many were found. Nothing here can allow or run any of it.
 */
export function SecurityDetails({ findings, scanComplete, onClose }: SecurityDetailsProps) {
  const closeRef = useRef<HTMLButtonElement>(null);
  // Opening the panel moves focus into it.
  useEffect(() => {
    closeRef.current?.focus();
  }, []);

  return (
    <aside
      id="security-details"
      aria-labelledby="security-details-title"
      className="flex w-[380px] max-w-full shrink-0 flex-col border-l bg-background max-[959px]:absolute max-[959px]:inset-y-0 max-[959px]:right-0 max-[959px]:z-20 max-[959px]:shadow-lg"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="flex items-center gap-2 border-b px-4 py-2">
        <h2 id="security-details-title" className="flex-1 text-sm font-semibold">
          {t.detailsTitle}
        </h2>
        <Button ref={closeRef} variant="ghost" size="icon-sm" aria-label={t.detailsClose} onClick={onClose}>
          <X />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
        <p className="text-sm text-muted-foreground">{t.detailsNote}</p>
        <ul className="mt-2 divide-y">
          {orderedFindings(findings).map((finding) => {
            const text = strings.findings[finding.kind];
            const leaky = LEAKY_KINDS.has(finding.kind);
            return (
              <li key={finding.kind} data-kind={finding.kind} className="py-3">
                <div className="flex items-baseline gap-2">
                  {leaky && <TriangleAlert aria-hidden className="size-4 shrink-0 self-center text-destructive" />}
                  <span className={leaky ? "flex-1 font-medium text-destructive" : "flex-1 font-medium"}>
                    {text.name}
                  </span>
                  <span className="text-sm text-muted-foreground tabular-nums">{t.detailsCount(finding.count)}</span>
                </div>
                <p className="mt-1 text-sm text-muted-foreground">{text.description}</p>
              </li>
            );
          })}
        </ul>
        {!scanComplete && (
          <p
            role="note"
            className="mt-3 rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100"
          >
            {t.scanIncomplete}
          </p>
        )}
      </div>
    </aside>
  );
}
