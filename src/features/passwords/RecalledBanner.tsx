// A file that opened with the password the app remembered for it (B2-26, ADR 0017): the user is
// told, and can have the password forgotten from here.

import { KeyRound, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { strings } from "@/i18n/zh-TW";

const t = strings.password.recalled;

type RecalledBannerProps = {
  /** Forgets the password the app remembers for this file. */
  onForget: () => void;
  /** Hides the notice; the password stays remembered. */
  onDismiss: () => void;
};

export function RecalledBanner({ onForget, onDismiss }: RecalledBannerProps) {
  return (
    <div
      role="region"
      aria-label={t.label}
      data-region="password-recalled"
      className="flex shrink-0 items-center gap-2 border-b border-sky-300 bg-sky-50 px-3 py-1.5 text-sm text-sky-950 dark:border-sky-800 dark:bg-sky-950 dark:text-sky-100"
    >
      <KeyRound className="size-4 shrink-0" aria-hidden />
      <p className="min-w-0 flex-1">{t.text}</p>
      <Button variant="outline" size="sm" onClick={onForget}>
        {t.forget}
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label={t.dismiss} onClick={onDismiss}>
        <X />
      </Button>
    </div>
  );
}
