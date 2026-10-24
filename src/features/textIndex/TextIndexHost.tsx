// Searching every document opened before (B3-21) belongs to the app, not to a screen: it is
// offered on the start screen and in every tab, and what it opens is a new tab.

import { useMemo, useState, type ReactNode } from "react";

import type { TextIndexApi } from "@/features/textIndex/api";
import { SearchAllDialog } from "@/features/textIndex/SearchAllDialog";
import { TextIndexContext } from "@/features/textIndex/useTextIndex";
import type { TabId } from "@/ipc/generated/contract";

type TextIndexHostProps = {
  api: TextIndexApi | undefined;
  /** A result was opened in `tab`: it is to show `page` (1-based) once its document is open. */
  onOpened: (tab: TabId, page: number) => void;
  children: ReactNode;
};

export function TextIndexHost({ api, onOpened, children }: TextIndexHostProps) {
  const [open, setOpen] = useState(false);
  const access = useMemo(() => (api ? { api, openSearchAll: () => setOpen(true) } : undefined), [api]);
  return (
    <TextIndexContext.Provider value={access}>
      {children}
      {api && (
        <SearchAllDialog
          open={open}
          onOpenChange={setOpen}
          api={api}
          onOpened={(tab, page) => {
            setOpen(false);
            onOpened(tab, page);
          }}
        />
      )}
    </TextIndexContext.Provider>
  );
}
