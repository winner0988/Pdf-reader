import { Info, TriangleAlert, X } from "lucide-react";
import { useEffect } from "react";

import { Button } from "@/components/ui/button";
import type { TabsNotice } from "@/features/tabs/model";
import { strings } from "@/i18n/zh-TW";
import { LIMITS } from "@/ipc/generated/contract";

/** How long a notice stays before it hides itself. */
const NOTICE_MS = 8000;

export function OpenNotice({ notice, onDismiss }: { notice: TabsNotice; onDismiss: () => void }) {
  useEffect(() => {
    const timer = window.setTimeout(onDismiss, NOTICE_MS);
    return () => window.clearTimeout(timer);
  }, [notice, onDismiss]);

  const failed = notice.kind === "failed";
  const text = failed
    ? strings.error.messages[notice.code] || strings.error.title
    : strings.tabs.tabLimit(LIMITS.maxTabs, notice.ignoredFiles);
  return (
    <div
      role={failed ? "alert" : "status"}
      className="fixed bottom-10 left-1/2 z-40 flex max-w-[calc(100%-2rem)] -translate-x-1/2 items-center gap-2 rounded-lg border bg-background px-4 py-2 text-sm shadow-md"
    >
      {failed ? (
        <TriangleAlert className="size-4 shrink-0 text-destructive" aria-hidden />
      ) : (
        <Info className="size-4 shrink-0 text-primary" aria-hidden />
      )}
      <span className="min-w-0 truncate">{text}</span>
      <Button variant="ghost" size="icon-sm" aria-label={strings.open.dismissNotice} onClick={onDismiss}>
        <X aria-hidden />
      </Button>
    </div>
  );
}
