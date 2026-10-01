// Changes an earlier run of the app left unsaved for the open file (B2-13, ADR 0013): the main
// process kept a journal of them, and the user decides whether they are made again.

import { HistoryIcon, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { strings } from "@/i18n/zh-TW";
import type { Recovery } from "@/ipc/generated/contract";

const t = strings.recovery;

type RecoveryBannerProps = {
  /** `available`: the edits can be made again; `stale`: the file changed since, so they cannot. */
  recovery: Exclude<Recovery, "none">;
  /** While an answer is on its way, the buttons wait. */
  busy: boolean;
  onRestore: () => void;
  onDiscard: () => void;
  /** Neither: the edits stay, and are offered again the next time the file opens. */
  onLater: () => void;
};

export function RecoveryBanner({ recovery, busy, onRestore, onDiscard, onLater }: RecoveryBannerProps) {
  return (
    <div
      role="region"
      aria-label={t.label}
      data-region="recovery"
      className="flex shrink-0 items-center gap-2 border-b border-sky-300 bg-sky-50 px-3 py-1.5 text-sm text-sky-950 dark:border-sky-800 dark:bg-sky-950 dark:text-sky-100"
    >
      <HistoryIcon className="size-4 shrink-0" aria-hidden />
      <p className="min-w-0 flex-1">{recovery === "available" ? t.available : t.stale}</p>
      {recovery === "available" && (
        <Button size="sm" disabled={busy} onClick={onRestore}>
          {t.restore}
        </Button>
      )}
      <Button variant="outline" size="sm" disabled={busy} onClick={onDiscard}>
        {t.discard}
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label={t.later} disabled={busy} onClick={onLater}>
        <X />
      </Button>
    </div>
  );
}
