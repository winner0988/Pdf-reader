import { useMemo, useState } from "react";

import { tauriExportApi, type ExportApi } from "@/features/export/api";
import { tauriOpenApi, type OpenApi } from "@/features/open/api";
import { OpenNotice } from "@/features/open/OpenNotice";
import { tauriLinksApi, type LinksApi } from "@/features/links/source";
import { tauriSearchApi, type SearchApi } from "@/features/search/useSearch";
import { tauriOutlineApi, useOutline, type OutlineApi } from "@/features/outline/useOutline";
import { tauriRecentApi, type RecentApi } from "@/features/recent/api";
import { tauriSavingApi, type SavingApi } from "@/features/saving/api";
import { UnsavedDialog } from "@/features/saving/UnsavedDialog";
import { tauriSettingsApi, type SettingsApi } from "@/features/settings/api";
import { SettingsProvider } from "@/features/settings/SettingsProvider";
import { DevDemoSwitcher } from "@/features/shell/DevDemoSwitcher";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { useShortcuts } from "@/features/shortcuts/useShortcuts";
import { tauriSystemApi, type SystemApi } from "@/features/system/defaultApp";
import { tauriTextApi, type TextApi } from "@/features/text/source";
import { docOf, isUnsaved, shellState, tabElementId, tabPanelId, type Tab } from "@/features/tabs/model";
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
  recentApi?: RecentApi;
  settingsApi?: SettingsApi;
  exportApi?: ExportApi;
  savingApi?: SavingApi;
};

export default function App({
  api = tauriOpenApi,
  renderApi = tauriRenderApi,
  outlineApi = tauriOutlineApi,
  searchApi = tauriSearchApi,
  linksApi = tauriLinksApi,
  textApi = tauriTextApi,
  systemApi = tauriSystemApi,
  recentApi = tauriRecentApi,
  settingsApi = tauriSettingsApi,
  exportApi = tauriExportApi,
  savingApi = tauriSavingApi,
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

  // Unsaved changes (B2-02): closing their tab, or the window, asks first.
  const [closing, setClosing] = useState<TabId | null>(null);
  const requestClose = (tab: TabId) => {
    const found = state.tabs.find((candidate) => candidate.tab === tab);
    if (found && isUnsaved(found)) setClosing(tab);
    else tabs.close(tab);
  };
  const closingTab = state.tabs.find((tab) => tab.tab === closing && isUnsaved(tab));
  const closeRequest = state.closeRequest
    ? state.tabs.filter((tab) => state.closeRequest?.includes(tab.tab) && isUnsaved(tab))
    : [];
  /** Saves each tab's document in turn; stops at the first that fails. */
  const saveAll = async (list: Tab[]) => {
    for (const tab of list) {
      const doc = docOf(tab);
      if (doc !== null) await savingApi.save(doc);
    }
  };

  return (
    <SettingsProvider api={settingsApi}>
      <div className="flex h-screen flex-col">
        {state.tabs.length > 0 && !demo && (
          <TabBar
            tabs={state.tabs}
            active={state.active}
            onActivate={tabs.activate}
            onClose={requestClose}
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
              recentApi={demo ? undefined : recentApi}
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
                recentApi={recentApi}
                exportApi={exportApi}
                savingApi={savingApi}
                onOpen={open}
                onClose={requestClose}
                onRetry={tabs.retry}
                onUnlock={tabs.unlock}
              />
            ))
          )}
        </div>
        {state.notice && <OpenNotice notice={state.notice} onDismiss={tabs.dismissNotice} />}
        <UnsavedDialog
          names={closingTab ? [closingTab.displayName] : null}
          onSave={async () => {
            if (!closingTab) return;
            await saveAll([closingTab]);
            tabs.close(closingTab.tab);
            setClosing(null);
          }}
          onDiscard={() => {
            if (closingTab) tabs.close(closingTab.tab);
            setClosing(null);
          }}
          onCancel={() => setClosing(null)}
        />
        <UnsavedDialog
          names={closeRequest.length > 0 ? closeRequest.map((tab) => tab.displayName) : null}
          onSave={async () => {
            await saveAll(closeRequest);
            await savingApi.closeWindow(false);
          }}
          onDiscard={() => {
            savingApi.closeWindow(true).catch(() => {});
          }}
          onCancel={tabs.dismissCloseRequest}
        />
        {import.meta.env.DEV && <DevDemoSwitcher onChange={setDemo} />}
      </div>
    </SettingsProvider>
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
  recentApi: RecentApi;
  exportApi: ExportApi;
  savingApi: SavingApi;
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
