import { ShieldAlert, X } from "lucide-react";
import type { Ref } from "react";

import { Button } from "@/components/ui/button";
import { bannerSummary } from "@/features/security-banner/summary";
import { strings } from "@/i18n/zh-TW";
import type { SecurityFinding } from "@/ipc/generated/contract";

const t = strings.banner;

type SecurityBannerProps = {
  findings: SecurityFinding[];
  scanComplete: boolean;
  detailsOpen: boolean;
  onToggleDetails: () => void;
  onDismiss: () => void;
  /** The details button, so focus can return to it when the panel closes. */
  detailsButtonRef?: Ref<HTMLButtonElement>;
};

/**
 * Shown when a document contains blocked content (or could not be scanned completely). There is
 * deliberately no way to allow or run anything (ADR 0001, 0002).
 */
export function SecurityBanner({
  findings,
  scanComplete,
  detailsOpen,
  onToggleDetails,
  onDismiss,
  detailsButtonRef,
}: SecurityBannerProps) {
  return (
    <div
      role="region"
      aria-label={t.label}
      data-region="banner"
      className="flex shrink-0 items-center gap-2 border-b border-amber-300 bg-amber-50 px-3 py-1.5 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100"
    >
      <ShieldAlert className="size-4 shrink-0" aria-hidden />
      <p className="min-w-0 flex-1">{bannerSummary(findings, scanComplete)}</p>
      <Button
        ref={detailsButtonRef}
        variant="outline"
        size="sm"
        aria-expanded={detailsOpen}
        aria-controls="security-details"
        onClick={onToggleDetails}
      >
        {t.details}
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label={t.dismiss} onClick={onDismiss}>
        <X />
      </Button>
    </div>
  );
}
