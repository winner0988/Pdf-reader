import { Info, X } from "lucide-react";
import { useEffect } from "react";

import { Button } from "@/components/ui/button";
import type { OpenNotice as Notice } from "@/features/open/session";
import { strings } from "@/i18n/zh-TW";

/** How long a notice stays before it hides itself. */
const NOTICE_MS = 8000;

export function OpenNotice({ notice, onDismiss }: { notice: Notice; onDismiss: () => void }) {
  useEffect(() => {
    const timer = window.setTimeout(onDismiss, NOTICE_MS);
    return () => window.clearTimeout(timer);
  }, [notice, onDismiss]);

  return (
    <div
      role="status"
      className="fixed bottom-10 left-1/2 z-40 flex max-w-[calc(100%-2rem)] -translate-x-1/2 items-center gap-2 rounded-lg border bg-background px-4 py-2 text-sm shadow-md"
    >
      <Info className="size-4 shrink-0 text-primary" aria-hidden />
      <span className="min-w-0 truncate">{strings.open.dropMultiple(notice.displayName)}</span>
      <Button variant="ghost" size="icon-sm" aria-label={strings.open.dismissNotice} onClick={onDismiss}>
        <X aria-hidden />
      </Button>
    </div>
  );
}
