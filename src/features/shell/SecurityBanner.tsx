import { ShieldAlert, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { bannerSummary } from "@/features/shell/format";
import type { SecurityFinding } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

const t = strings.banner;

type SecurityBannerProps = {
  findings: SecurityFinding[];
  onDismiss: () => void;
};

/** Shown when a document contains blocked content. The details panel arrives with MVP-11. */
export function SecurityBanner({ findings, onDismiss }: SecurityBannerProps) {
  return (
    <div
      role="region"
      aria-label={t.label}
      data-region="banner"
      className="flex shrink-0 items-center gap-2 border-b border-amber-300 bg-amber-50 px-3 py-1.5 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100"
    >
      <ShieldAlert className="size-4 shrink-0" aria-hidden />
      <p className="min-w-0 flex-1">{bannerSummary(findings)}</p>
      <Button variant="outline" size="sm" disabled>
        {t.details}
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label={t.dismiss} onClick={onDismiss}>
        <X />
      </Button>
    </div>
  );
}
