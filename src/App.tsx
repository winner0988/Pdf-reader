import { useEffect, useMemo, useState } from "react";

import { tauriExportApi, type ExportApi } from "@/features/export/api";
import { tauriOpenApi, type OpenApi } from "@/features/open/api";
import { OpenNotice } from "@/features/open/OpenNotice";
import { tauriAnnotationsApi, type AnnotationsApi } from "@/features/annotations/source";
import { FieldEdits } from "@/features/forms/edits";
import { tauriFormsApi, type FormsApi } from "@/features/forms/source";
import { tauriLinksApi, type LinksApi } from "@/features/links/source";
import { tauriOcrApi, type OcrApi } from "@/features/ocr/api";
import type { OcrTab } from "@/features/ocr/model";
import { useOcr } from "@/features/ocr/useOcr";
import { tauriSearchApi, type SearchApi } from "@/features/search/useSearch";
import { tauriOutlineApi, useOutline, type OutlineApi } from "@/features/outline/useOutline";
import { tauriRecentApi, type RecentApi } from "@/features/recent/api";
import { tauriSavingApi, type SavingApi } from "@/features/saving/api";
import { UnsavedDialog } from "@/features/saving/UnsavedDialog";
import { tauriSettingsApi, type SettingsApi } from "@/features/settings/api";
import { SettingsProvider } from "@/features/settings/SettingsProvider";
import { tauriUpdatesApi, type UpdatesApi } from "@/features/settings/updates";
import { DevDemoSwitcher } from "@/features/shell/DevDemoSwitcher";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { useShortcuts } from "@/features/shortcuts/useShortcuts";
import { tauriSystemApi, type SystemApi } from "@/features/system/defaultApp";
import { tauriTextApi, type TextApi } from "@/features/text/source";
import { docOf, isUnsaved, shellState, tabElementId, tabPanelId, type Tab } from "@/features/tabs/model";
import { TabBar } from "@/features/tabs/TabBar";
import { tauriEditingApi, type EditingApi } from "@/features/thumbnails/api";
import { useTabs } from "@/features/tabs/useTabs";
import { createPageRenderer, tauriRenderApi, type PageRenderer, type RenderApi } from "@/features/viewer/renderer";
import type { DocumentInfo, TabId } from "@/ipc/generated/contract";

type AppProps = {
  api?: OpenApi;
  renderApi?: RenderApi;
  outlineApi?: OutlineApi;
  searchApi?: SearchApi;
  linksApi?: LinksApi;
  textApi?: TextApi;
  annotationsApi?: AnnotationsApi;
  formsApi?: FormsApi;
  systemApi?: SystemApi;
  recentApi?: RecentApi;
  settingsApi?: SettingsApi;
  exportApi?: ExportApi;
  savingApi?: SavingApi;
  editingApi?: EditingApi;
  updatesApi?: UpdatesApi;
  ocrApi?: OcrApi;
};

export default function App({
  api = tauriOpenApi,
  renderApi = tauriRenderApi,
  outlineApi = tauriOutlineApi,
  searchApi = tauriSearchApi,
  linksApi = tauriLinksApi,
  textApi = tauriTextApi,
  annotationsApi = tauriAnnotationsApi,
  formsApi = tauriFormsApi,
  systemApi = tauriSystemApi,
  recentApi = tauriRecentApi,
  settingsApi = tauriSettingsApi,
  exportApi = tauriExportApi,
  savingApi = tauriSavingApi,
  editingApi = tauriEditingApi,
  updatesApi = tauriUpdatesApi,
  ocrApi = tauriOcrApi,
}: AppProps) {
  // How recognising the text of each tab's scanned pages goes arrives with the open events (B2-10).
  const ocr = useOcr();
  const tabs = useTabs(api, ocr.handle);
  const { state } = tabs;
  // A closed tab's recognising is forgotten.
  const { state: ocrState, forget: forgetOcr } = ocr;
  useEffect(() => {
    for (const tab of ocrState.keys()) {
      if (!state.tabs.some((open) => open.tab === tab)) forgetOcr(tab);
    }
  }, [state.tabs, ocrState, forgetOcr]);
  // The values filled into forms, on their way to the documents (B2-09): closing a tab waits for them.
  const [fieldEdits] = useState(() => new FieldEdits());
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
    // A value still being typed in a field is a change too, and is sent first (B2-09).
    fieldEdits.whenSettled(() => {
      const known = tabs.latestInfo(tab);
      const found = state.tabs.find((candidate) => candidate.tab === tab);
      if (known?.unsaved ?? (found && isUnsaved(found))) setClosing(tab);
      else tabs.close(tab);
    });
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
              updatesApi={demo ? undefined : updatesApi}
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
                annotationsApi={annotationsApi}
                formsApi={formsApi}
                systemApi={systemApi}
                recentApi={recentApi}
                exportApi={exportApi}
                savingApi={savingApi}
                editingApi={editingApi}
                updatesApi={updatesApi}
                ocrApi={ocrApi}
                ocr={ocr.state.get(tab.tab)}
                fieldEdits={fieldEdits}
                latestInfo={tabs.latestInfo}
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
  annotationsApi: AnnotationsApi;
  formsApi: FormsApi;
  systemApi: SystemApi;
  recentApi: RecentApi;
  exportApi: ExportApi;
  savingApi: SavingApi;
  editingApi: EditingApi;
  updatesApi: UpdatesApi;
  ocrApi: OcrApi;
  ocr: OcrTab | undefined;
  fieldEdits: FieldEdits;
  latestInfo: (tab: TabId) => DocumentInfo | undefined;
  onOpen: () => void;
  onClose: (tab: TabId) => void;
  onRetry: (tab: TabId) => void;
  onUnlock: (tab: TabId, password: string) => void;
};

/**
 * One tab's reader. Every tab stays mounted and hidden tabs are only hidden, so each keeps its
 * page, zoom, rotation, sidebar and search while another is shown (MVP-14).
 */
function TabPane({ tab, active, outlineApi, latestInfo, onClose, onRetry, onUnlock, ...shell }: TabPaneProps) {
  const outline = useOutline(outlineApi, docOf(tab), tab.content.kind === "open" && tab.content.hasOutline);
  return (
    <div role="tabpanel" id={tabPanelId(tab.tab)} aria-labelledby={tabElementId(tab.tab)} hidden={!active} className="h-full">
      <ReaderShell
        {...shell}
        state={shellState(tab)}
        active={active}
        outline={outline}
        latest={() => latestInfo(tab.tab)}
        onClose={() => onClose(tab.tab)}
        onRetry={() => onRetry(tab.tab)}
        onUnlock={(password) => onUnlock(tab.tab, password)}
      />
    </div>
  );
}
