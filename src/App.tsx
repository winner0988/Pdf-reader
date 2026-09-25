import { useMemo, useState } from "react";

import { tauriOpenApi, type OpenApi } from "@/features/open/api";
import { OpenNotice } from "@/features/open/OpenNotice";
import { tauriLinksApi, type LinksApi } from "@/features/links/source";
import { tauriSearchApi, type SearchApi } from "@/features/search/useSearch";
import { tauriOutlineApi, useOutline, type OutlineApi } from "@/features/outline/useOutline";
import { DevDemoSwitcher } from "@/features/shell/DevDemoSwitcher";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { useShortcuts } from "@/features/shortcuts/useShortcuts";
import { tauriSystemApi, type SystemApi } from "@/features/system/defaultApp";
import { tauriTextApi, type TextApi } from "@/features/text/source";
import { docOf, shellState, tabElementId, tabPanelId, type Tab } from "@/features/tabs/model";
import { TabBar } from "@/features/tabs/TabBar";
import { useTabs } from "@/features/tabs/useTabs";
import { createPageRenderer, tauriRenderApi, type PageRenderer, type RenderApi } from "@/features/viewer/renderer";
import type { TabId } from "@/ipc/generated/contract";

type AppProps = {
  api?: OpenApi;
  renderApi?: RenderApi;
  outlineApi?: OutlineApi;
  searchApi?: SearchApi;
  linksApi?: LinksApi;
  textApi?: TextApi;
  systemApi?: SystemApi;
};

export default function App({
  api = tauriOpenApi,
  renderApi = tauriRenderApi,
  outlineApi = tauriOutlineApi,
  searchApi = tauriSearchApi,
  linksApi = tauriLinksApi,
  textApi = tauriTextApi,
  systemApi = tauriSystemApi,
}: AppProps) {
  const tabs = useTabs(api);
  const { state } = tabs;
  const renderer = useMemo(() => createPageRenderer(renderApi), [renderApi]);
  // Development only: fake states for working on the UI without the main process.
  const [demo, setDemo] = useState<ShellState | null>(null);

  useShortcuts({
    nextTab: () => tabs.step(1),
    previousTab: () => tabs.step(-1),
  });

  const open = () => {
    setDemo(null);
    tabs.open();
  };

  return (
    <div className="flex h-screen flex-col">
      {state.tabs.length > 0 && !demo && (
        <TabBar
          tabs={state.tabs}
          active={state.active}
          onActivate={tabs.activate}
          onClose={tabs.close}
          onOpen={open}
        />
      )}
      <div className="min-h-0 flex-1">
        {state.tabs.length === 0 || demo ? (
          <ReaderShell
            state={demo ?? { kind: "empty" }}
            dropActive={state.dragActive}
            renderer={renderer}
            systemApi={systemApi}
            onOpen={open}
            onClose={() => setDemo(null)}
          />
        ) : (
          state.tabs.map((tab) => (
            <TabPane
              key={tab.tab}
              tab={tab}
              active={tab.tab === state.active}
              dropActive={state.dragActive}
              renderer={renderer}
              outlineApi={outlineApi}
              searchApi={searchApi}
              linksApi={linksApi}
              textApi={textApi}
              systemApi={systemApi}
              onOpen={open}
              onClose={tabs.close}
              onRetry={tabs.retry}
              onUnlock={tabs.unlock}
            />
          ))
        )}
      </div>
      {state.notice && <OpenNotice notice={state.notice} onDismiss={tabs.dismissNotice} />}
      {import.meta.env.DEV && <DevDemoSwitcher onChange={setDemo} />}
    </div>
  );
}

type TabPaneProps = {
  tab: Tab;
  active: boolean;
  dropActive: boolean;
  renderer: PageRenderer;
  outlineApi: OutlineApi;
  searchApi: SearchApi;
  linksApi: LinksApi;
  textApi: TextApi;
  systemApi: SystemApi;
  onOpen: () => void;
  onClose: (tab: TabId) => void;
  onRetry: (tab: TabId) => void;
  onUnlock: (tab: TabId, password: string) => void;
};

/**
 * One tab's reader. Every tab stays mounted and hidden tabs are only hidden, so each keeps its
 * page, zoom, rotation, sidebar and search while another is shown (MVP-14).
 */
function TabPane({ tab, active, outlineApi, onClose, onRetry, onUnlock, ...shell }: TabPaneProps) {
  const outline = useOutline(outlineApi, docOf(tab), tab.content.kind === "open" && tab.content.hasOutline);
  return (
    <div role="tabpanel" id={tabPanelId(tab.tab)} aria-labelledby={tabElementId(tab.tab)} hidden={!active} className="h-full">
      <ReaderShell
        {...shell}
        state={shellState(tab)}
        active={active}
        outline={outline}
        onClose={() => onClose(tab.tab)}
        onRetry={() => onRetry(tab.tab)}
        onUnlock={(password) => onUnlock(tab.tab, password)}
      />
    </div>
  );
}
