import { ShieldAlert, ShieldCheck, ShieldX, X } from "lucide-react";
import { useEffect, useId, useRef } from "react";

import { Button } from "@/components/ui/button";
import { categoryOf, type Category } from "@/features/signatures/summary";
import { strings } from "@/i18n/zh-TW";
import type { SignatureInfo, SignatureReport } from "@/ipc/generated/contract";

const t = strings.signatures;

const AMBER = "text-amber-600 dark:text-amber-400";

const ICONS: Record<Category, { Icon: typeof ShieldCheck; classes: string }> = {
  valid: { Icon: ShieldCheck, classes: "text-emerald-600 dark:text-emerald-400" },
  unconfirmed: { Icon: ShieldAlert, classes: AMBER },
  changed: { Icon: ShieldAlert, classes: AMBER },
  unverifiable: { Icon: ShieldAlert, classes: AMBER },
  invalid: { Icon: ShieldX, classes: "text-destructive" },
};

type SignatureDetailsProps = {
  /** What the banner's details button controls: every tab has its own (MVP-14). */
  id: string;
  report: SignatureReport;
  onClose: () => void;
};

/**
 * The signatures' details panel (docs/ux/screen-map.md, section 5): one entry for each signature
 * with its state in words, who signed and when they say they did.
 */
export function SignatureDetails({ id, report, onClose }: SignatureDetailsProps) {
  const closeRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  // Opening the panel moves focus into it.
  useEffect(() => {
    closeRef.current?.focus();
  }, []);

  return (
    <aside
      id={id}
      aria-labelledby={titleId}
      className="flex w-[380px] max-w-full shrink-0 flex-col border-l bg-background max-[959px]:absolute max-[959px]:inset-y-0 max-[959px]:right-0 max-[959px]:z-20 max-[959px]:shadow-lg"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="flex items-center gap-2 border-b px-4 py-2">
        <h2 id={titleId} className="flex-1 text-sm font-semibold">
          {t.detailsTitle}
        </h2>
        <Button ref={closeRef} variant="ghost" size="icon-sm" aria-label={t.detailsClose} onClick={onClose}>
          <X />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
        <p className="text-sm text-muted-foreground">{t.detailsNote}</p>
        <ul className="mt-2 divide-y">
          {report.signatures.map((signature, index) => (
            <SignatureEntry key={index} signature={signature} index={index + 1} />
          ))}
        </ul>
        {report.truncated && (
          <p
            role="note"
            className="mt-3 rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100"
          >
            {t.truncated(report.signatures.length)}
          </p>
        )}
      </div>
    </aside>
  );
}

function SignatureEntry({ signature, index }: { signature: SignatureInfo; index: number }) {
  const category = categoryOf(signature);
  const { Icon, classes } = ICONS[category];
  const explanation =
    category === "unverifiable"
      ? signature.reason
        ? t.reason[signature.reason]
        : null
      : t.explanation[category];
  // A certifying signature that allows nothing, and a document that changed: said outright.
  const broken = category === "changed" && signature.certification === "noChanges";
  return (
    <li data-category={category} className="py-3">
      <div className="flex items-baseline gap-2">
        <Icon aria-hidden className={`size-4 shrink-0 self-center ${classes}`} />
        <span className="flex-1 font-medium">{signature.fieldName ?? t.unnamed(index)}</span>
      </div>
      <p className="mt-1 text-sm font-medium">{t.status[category]}</p>
      {explanation && <p className="mt-1 text-sm text-muted-foreground">{explanation}</p>}
      {broken && (
        <p role="note" className="mt-1 text-sm font-medium text-destructive">
          {t.certificationBroken}
        </p>
      )}
      <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
        {signature.signer !== null && <Row term={t.signer} text={signature.signer} />}
        {signature.claimedTime !== null && <Row term={t.time} text={signature.claimedTime} note={t.timeNote} />}
        {signature.certification !== null && (
          <Row term={t.certification} text={t.certificationLevel[signature.certification]} />
        )}
      </dl>
    </li>
  );
}

function Row({ term, text, note }: { term: string; text: string; note?: string }) {
  return (
    <>
      <dt className="text-muted-foreground">{term}</dt>
      <dd className="min-w-0 break-words">
        {text}
        {note && <span className="block text-xs text-muted-foreground">{note}</span>}
      </dd>
    </>
  );
}
