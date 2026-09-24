import { useMemo, useState } from "react";

import { tauriOpenApi, type OpenApi } from "@/features/open/api";
import { OpenNotice } from "@/features/open/OpenNotice";
import { useOpenSession } from "@/features/open/useOpenSession";
import { tauriOutlineApi, useOutline, type OutlineApi } from "@/features/outline/useOutline";
import { DevDemoSwitcher } from "@/features/shell/DevDemoSwitcher";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { createPageRenderer, tauriRenderApi, type RenderApi } from "@/features/viewer/renderer";

type AppProps = { api?: OpenApi; renderApi?: RenderApi; outlineApi?: OutlineApi };

export default function App({
  api = tauriOpenApi,
  renderApi = tauriRenderApi,
  outlineApi = tauriOutlineApi,
}: AppProps) {
  const { session, open, retry, close, dismissNotice } = useOpenSession(api);
  const renderer = useMemo(() => createPageRenderer(renderApi), [renderApi]);
  const outline = useOutline(outlineApi, session.doc, session.hasOutline);
  // Development only: fake states for working on the UI without the main process.
  const [demo, setDemo] = useState<ShellState | null>(null);

  return (
    <>
      <ReaderShell
        state={demo ?? session.shell}
        dropActive={session.dragActive}
        renderer={renderer}
        outline={demo ? undefined : outline}
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
