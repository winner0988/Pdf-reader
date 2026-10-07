import { ShieldAlert, ShieldCheck, ShieldX, X } from "lucide-react";
import type { Ref } from "react";

import { Button } from "@/components/ui/button";
import { bannerSummary, worstCategory, type Category } from "@/features/signatures/summary";
import { strings } from "@/i18n/zh-TW";
import type { SignatureReport } from "@/ipc/generated/contract";

const t = strings.signatures;

type Tone = { classes: string; Icon: typeof ShieldCheck };

const AMBER = "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100";

/** The banner's colour follows the worst signature: only an all-clear is green. */
const TONES: Record<Category, Tone> = {
  valid: {
    classes:
      "border-emerald-300 bg-emerald-50 text-emerald-900 dark:border-emerald-800 dark:bg-emerald-950 dark:text-emerald-100",
    Icon: ShieldCheck,
  },
  unconfirmed: { classes: AMBER, Icon: ShieldAlert },
  changed: { classes: AMBER, Icon: ShieldAlert },
  unverifiable: { classes: AMBER, Icon: ShieldAlert },
  invalid: {
    classes: "border-red-300 bg-red-50 text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100",
    Icon: ShieldX,
  },
};

type SignatureBannerProps = {
  report: SignatureReport;
  detailsOpen: boolean;
  /** The details panel's id: every tab has its own (MVP-14). */
  detailsId: string;
  onToggleDetails: () => void;
  onDismiss: () => void;
  /** The details button, so focus can return to it when the panel closes. */
  detailsButtonRef?: Ref<HTMLButtonElement>;
};

/**
 * Shown when a document has digital signatures (B2-14): what they add up to, and the way to the
 * details. Offline, and never more than the signatures say.
 */
export function SignatureBanner({
  report,
  detailsOpen,
  detailsId,
  onToggleDetails,
  onDismiss,
  detailsButtonRef,
}: SignatureBannerProps) {
  const worst = worstCategory(report);
  const { classes, Icon } = TONES[worst];
  return (
    <div
      role="region"
      aria-label={t.label}
      data-region="banner"
      data-signatures={worst}
      className={`flex shrink-0 items-center gap-2 border-b px-3 py-1.5 text-sm ${classes}`}
    >
      <Icon className="size-4 shrink-0" aria-hidden />
      <p className="min-w-0 flex-1">{bannerSummary(report)}</p>
      <Button
        ref={detailsButtonRef}
        variant="outline"
        size="sm"
        aria-expanded={detailsOpen}
        aria-controls={detailsId}
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
