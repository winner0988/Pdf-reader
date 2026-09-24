import { FileText, Loader2, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { RETRYABLE_ERRORS, type ErrorCode } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

export function EmptyState({ onOpen }: { onOpen: () => void }) {
  const t = strings.empty;
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 p-8 text-center">
      <FileText className="size-16 text-muted-foreground" aria-hidden />
      <h1 className="text-2xl font-semibold">{t.title}</h1>
      <Button size="lg" autoFocus onClick={onOpen}>
        {t.openButton}
      </Button>
      <p className="text-sm text-muted-foreground">{t.dropHint}</p>
      <p className="mt-6 rounded-md bg-muted px-4 py-2 text-xs text-muted-foreground">{t.privacyNote}</p>
    </div>
  );
}

/** Appears only after `delayMs`, so fast opens do not flash (docs/ux/screen-map.md, section 2). */
export function LoadingState({ displayName, delayMs = 300 }: { displayName: string; delayMs?: number }) {
  const [visible, setVisible] = useState(delayMs <= 0);
  useEffect(() => {
    if (delayMs <= 0) return;
    const timer = window.setTimeout(() => setVisible(true), delayMs);
    return () => window.clearTimeout(timer);
  }, [delayMs]);

  if (!visible) return null;
  return (
    <div role="status" className="flex h-full items-center justify-center p-8">
      <div className="flex items-center gap-3 rounded-lg border bg-background px-5 py-4 shadow-sm">
        <Loader2 className="size-5 animate-spin text-primary motion-reduce:animate-none" aria-hidden />
        <span>{strings.loading(displayName)}</span>
      </div>
    </div>
  );
}

type ErrorStateProps = {
  code: ErrorCode;
  displayName?: string;
  onOpen: () => void;
  onRetry?: () => void;
};

export function ErrorState({ code, displayName, onOpen, onRetry }: ErrorStateProps) {
  const t = strings.error;
  const canRetry = RETRYABLE_ERRORS.has(code) && onRetry !== undefined;
  return (
    <div role="alert" className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
      <TriangleAlert className="size-12 text-destructive" aria-hidden />
      <h1 className="text-xl font-semibold">{t.title}</h1>
      <p className="text-muted-foreground">{t.messages[code]}</p>
      {displayName && <p className="text-sm text-muted-foreground">{displayName}</p>}
      <div className="mt-2 flex gap-2">
        <Button onClick={onOpen}>{t.openAnother}</Button>
        {canRetry && (
          <Button variant="outline" onClick={onRetry}>
            {t.retry}
          </Button>
        )}
      </div>
    </div>
  );
}
