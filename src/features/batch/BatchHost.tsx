// The batch dialog (B3-04, ADR 0005) belongs to the app, not to a screen. A batch goes on while
// the first document opens over the start screen, while the tab it was started from is closed,
// and with the window closed and shown again: the dialog has to be there to say how it went.

import { useMemo, useState, type ReactNode } from "react";

import type { BatchApi } from "@/features/batch/api";
import { BatchDialog } from "@/features/batch/BatchDialog";
import { OpenBatch } from "@/features/batch/useOpenBatch";

export function BatchHost({ api, children }: { api: BatchApi | undefined; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const show = useMemo(() => (api ? () => setOpen(true) : undefined), [api]);
  return (
    <OpenBatch.Provider value={show}>
      {children}
      {api && <BatchDialog open={open} onOpenChange={setOpen} api={api} />}
    </OpenBatch.Provider>
  );
}
