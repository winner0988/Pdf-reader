import { useState } from "react";

import { tauriOpenApi, type OpenApi } from "@/features/open/api";
import { OpenNotice } from "@/features/open/OpenNotice";
import { useOpenSession } from "@/features/open/useOpenSession";
import { DevDemoSwitcher } from "@/features/shell/DevDemoSwitcher";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";

export default function App({ api = tauriOpenApi }: { api?: OpenApi }) {
  const { session, open, retry, close, dismissNotice } = useOpenSession(api);
  // Development only: fake states for working on the UI without the main process.
  const [demo, setDemo] = useState<ShellState | null>(null);

  return (
    <>
      <ReaderShell
        state={demo ?? session.shell}
        dropActive={session.dragActive}
        onOpen={() => {
          setDemo(null);
          open();
        }}
        onClose={() => {
          setDemo(null);
          close();
        }}
        onRetry={retry}
      />
      {session.notice && <OpenNotice notice={session.notice} onDismiss={dismissNotice} />}
      {import.meta.env.DEV && <DevDemoSwitcher onChange={setDemo} />}
    </>
  );
}
