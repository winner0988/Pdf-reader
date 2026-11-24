// Status notice for creating a PDF from images passed via File Explorer's context menu
// (`--create-pdf-from`, B5-13, #450): shows collecting count, creating progress, completion, or
// failure using the same wording as CreatePdfDialog. Paths never reach the frontend (ADR 0008).

import { CheckCircle2, Loader2, TriangleAlert, X } from "lucide-react";
import { useEffect } from "react";

import { Button } from "@/components/ui/button";
import { createFailure } from "@/features/create/messages";
import { strings } from "@/i18n";
import type { ExplorerCreateStatus } from "@/ipc/generated/contract";
import { cn } from "@/lib/utils";

/** How long a finished or failed notice stays before hiding itself (matches OpenNotice). */
const NOTICE_MS = 8000;

type ExplorerCreateNoticeProps = {
  status: ExplorerCreateStatus;
  onDismiss: () => void;
  /** Shift up when OpenNotice is shown at the same time so the two never overlap. */
  stacked?: boolean;
};

export function ExplorerCreateNotice({ status, onDismiss, stacked = false }: ExplorerCreateNoticeProps) {
  const terminal = status.phase === "done" || status.phase === "failed";

  useEffect(() => {
    if (!terminal) return;
    const timer = window.setTimeout(onDismiss, NOTICE_MS);
    return () => window.clearTimeout(timer);
  }, [status, terminal, onDismiss]);

  if (status.phase === "idle") return null;

  const t = strings.create;
  let text: string;
  switch (status.phase) {
    case "collecting":
      text = t.explorerCollecting(status.count, status.skipped);
      break;
    case "creating":
      text = t.explorerCreating(status.count);
      break;
    case "done":
      text =
        status.skipped > 0
          ? t.explorerDoneSkipped(status.pages, status.skipped)
          : t.done(status.pages);
      break;
    case "failed":
      text = createFailure({ code: status.code, message: "" });
      break;
  }

  const failed = status.phase === "failed";
  const busy = status.phase === "collecting" || status.phase === "creating";

  return (
    <div
      role={failed ? "alert" : "status"}
      className={cn(
        "fixed left-1/2 z-40 flex max-w-[calc(100%-2rem)] -translate-x-1/2 items-center gap-2 rounded-lg border bg-background px-4 py-2 text-sm shadow-md",
        stacked ? "bottom-24" : "bottom-10",
      )}
    >
      {busy ? (
        <Loader2 className="size-4 shrink-0 animate-spin text-primary motion-reduce:animate-none" aria-hidden />
      ) : failed ? (
        <TriangleAlert className="size-4 shrink-0 text-destructive" aria-hidden />
      ) : (
        <CheckCircle2 className="size-4 shrink-0 text-primary" aria-hidden />
      )}
      <span className="min-w-0 truncate">{text}</span>
      {terminal && (
        <Button variant="ghost" size="icon-sm" aria-label={strings.open.dismissNotice} onClick={onDismiss}>
          <X aria-hidden />
        </Button>
      )}
    </div>
  );
}
