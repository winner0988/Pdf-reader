import { useEffect, useMemo, useState } from "react";

import { tauriBatchApi, type BatchApi } from "@/features/batch/api";
import { BatchHost } from "@/features/batch/BatchHost";
import { tauriCreateApi, type CreateApi } from "@/features/create/api";
import { tauriExportApi, type ExportApi } from "@/features/export/api";
import { tauriOpenApi, type OpenApi } from "@/features/open/api";
import { OpenNotice } from "@/features/open/OpenNotice";
import { tauriAnnotationsApi, type AnnotationsApi } from "@/features/annotations/source";
import { FieldEdits } from "@/features/forms/edits";
import { tauriFormsApi, type FormsApi } from "@/features/forms/source";
import { tauriLinksApi, type LinksApi } from "@/features/links/source";
import { tauriSignaturesApi, useSignatures, type SignaturesApi } from "@/features/signatures/useSignatures";
import { tauriOcrApi, type OcrApi } from "@/features/ocr/api";
import type { OcrTab } from "@/features/ocr/model";
import { useOcr } from "@/features/ocr/useOcr";
import { tauriSearchApi, type SearchApi } from "@/features/search/useSearch";
import { tauriOutlineApi, useOutline, type OutlineApi } from "@/features/outline/useOutline";
import { tauriRecentApi, type RecentApi } from "@/features/recent/api";
import { tauriSavingApi, type SavingApi } from "@/features/saving/api";
import { tauriSensitiveApi, type SensitiveApi } from "@/features/sensitive/api";
import { tauriSigningApi, type SigningApi } from "@/features/signing/api";
import { UnsavedDialog } from "@/features/saving/UnsavedDialog";
import { tauriSettingsApi, type SettingsApi } from "@/features/settings/api";
import { SettingsProvider } from "@/features/settings/SettingsProvider";
import { tauriFontsApi, type FontsApi } from "@/features/settings/fontsApi";
import type { PasswordsApi } from "@/features/settings/PasswordsSection";
import { tauriUpdatesApi, type UpdatesApi } from "@/features/settings/updates";
import { DevDemoSwitcher } from "@/features/shell/DevDemoSwitcher";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { useShortcuts } from "@/features/shortcuts/useShortcuts";
import { tauriSystemApi, type SystemApi } from "@/features/system/defaultApp";
import { tauriTextApi, type TextApi } from "@/features/text/source";
import { tauriTextIndexApi, type TextIndexApi } from "@/features/textIndex/api";
import { TextIndexHost } from "@/features/textIndex/TextIndexHost";
import { docOf, isUnsaved, shellState, tabElementId, tabPanelId, type Tab } from "@/features/tabs/model";
import { TabBar } from "@/features/tabs/TabBar";
import { tauriEditingApi, type EditingApi } from "@/features/thumbnails/api";
import { useTabs } from "@/features/tabs/useTabs";
import { createPageRenderer, tauriRenderApi, type PageRenderer, type RenderApi } from "@/features/viewer/renderer";
import type { CaptureProtection, DocumentInfo, TabId } from "@/ipc/generated/contract";

type AppProps = {
  api?: OpenApi;
  renderApi?: RenderApi;
  outlineApi?: OutlineApi;
  searchApi?: SearchApi;
  linksApi?: LinksApi;
  textApi?: TextApi;
  annotationsApi?: AnnotationsApi;
  formsApi?: FormsApi;
  signaturesApi?: SignaturesApi;
  systemApi?: SystemApi;
  recentApi?: RecentApi;
  settingsApi?: SettingsApi;
  exportApi?: ExportApi;
  savingApi?: SavingApi;
  batchApi?: BatchApi;
  createApi?: CreateApi;
  sensitiveApi?: SensitiveApi;
  signingApi?: SigningApi;
  editingApi?: EditingApi;
  updatesApi?: UpdatesApi;
  fontsApi?: FontsApi;
  ocrApi?: OcrApi;
  textIndexApi?: TextIndexApi;
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
  signaturesApi = tauriSignaturesApi,
  systemApi = tauriSystemApi,
  recentApi = tauriRecentApi,
  settingsApi = tauriSettingsApi,
  exportApi = tauriExportApi,
  savingApi = tauriSavingApi,
  batchApi = tauriBatchApi,
  createApi = tauriCreateApi,
  sensitiveApi = tauriSensitiveApi,
  signingApi = tauriSigningApi,
  editingApi = tauriEditingApi,
  updatesApi = tauriUpdatesApi,
  fontsApi = tauriFontsApi,
  ocrApi = tauriOcrApi,
  textIndexApi = tauriTextIndexApi,
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
  // A result of the search of every document opened in a new tab (B3-21): the page it is to show.
  const [jump, setJump] = useState<{ tab: TabId; page: number } | null>(null);

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
  // The window is being closed and no tab is known to have unsaved changes (#153): a value still
  // being typed in a field is one only this page knows of, so it is sent first. The main process
  // then closes the window, or asks again (naming the tabs) if the value made a change.
  const noTabKnown = state.closeRequest !== null && state.closeRequest.length === 0;
  const { dismissCloseRequest } = tabs;
  useEffect(() => {
    if (!noTabKnown) return;
    dismissCloseRequest();
    fieldEdits.whenSettled(() => {
      savingApi.closeWindow(false).catch(() => {});
    });
  }, [noTabKnown, dismissCloseRequest, fieldEdits, savingApi]);
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
      <TextIndexHost api={demo ? undefined : textIndexApi} onOpened={(tab, page) => setJump({ tab, page })}>
      <BatchHost api={demo ? undefined : batchApi}>
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
                ocrApi={demo ? undefined : ocrApi}
                passwordsApi={demo ? undefined : api}
                createApi={demo ? undefined : createApi}
                sensitiveApi={demo ? undefined : sensitiveApi}
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
                  signaturesApi={signaturesApi}
                  systemApi={systemApi}
                  recentApi={recentApi}
                  exportApi={exportApi}
                  savingApi={savingApi}
                  createApi={createApi}
                  sensitiveApi={sensitiveApi}
                  signingApi={signingApi}
                  editingApi={editingApi}
                  updatesApi={updatesApi}
                  fontsApi={fontsApi}
                  ocrApi={ocrApi}
                  passwordsApi={api}
                  ocr={ocr.state.get(tab.tab)}
                  fieldEdits={fieldEdits}
                  captureProtection={state.captureProtection}
                  jumpToPage={jump?.tab === tab.tab ? jump.page : undefined}
                  onJumped={() => setJump(null)}
                  latestInfo={tabs.latestInfo}
                  onOpen={open}
                  onClose={requestClose}
                  onRetry={tabs.retry}
                  onUnlock={tabs.unlock}
                  onForgetPassword={tabs.forgetPassword}
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
      </BatchHost>
      </TextIndexHost>
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
  signaturesApi: SignaturesApi;
  systemApi: SystemApi;
  recentApi: RecentApi;
  exportApi: ExportApi;
  savingApi: SavingApi;
  createApi: CreateApi;
  sensitiveApi: SensitiveApi;
  signingApi: SigningApi;
  editingApi: EditingApi;
  updatesApi: UpdatesApi;
  fontsApi: FontsApi;
  ocrApi: OcrApi;
  passwordsApi: PasswordsApi;
  ocr: OcrTab | undefined;
  fieldEdits: FieldEdits;
  captureProtection: CaptureProtection;
  jumpToPage: number | undefined;
  onJumped: () => void;
  latestInfo: (tab: TabId) => DocumentInfo | undefined;
  onOpen: () => void;
  onClose: (tab: TabId) => void;
  onRetry: (tab: TabId) => void;
  onUnlock: (tab: TabId, password: string, remember: boolean) => void;
  onForgetPassword: (tab: TabId) => void;
};

/**
 * One tab's reader. Every tab stays mounted and hidden tabs are only hidden, so each keeps its
 * page, zoom, rotation, sidebar and search while another is shown (MVP-14).
 */
function TabPane({
  tab,
  active,
  outlineApi,
  signaturesApi,
  latestInfo,
  onClose,
  onRetry,
  onUnlock,
  onForgetPassword,
  ...shell
}: TabPaneProps) {
  const outline = useOutline(
    outlineApi,
    docOf(tab),
    tab.content.kind === "open" && tab.content.hasOutline,
    tab.content.kind === "open" ? tab.content.document.session : undefined,
  );
  const signatures = useSignatures(
    signaturesApi,
    docOf(tab),
    tab.content.kind === "open" ? tab.content.document.session : undefined,
    isUnsaved(tab),
  );
  return (
    <div role="tabpanel" id={tabPanelId(tab.tab)} aria-labelledby={tabElementId(tab.tab)} hidden={!active} className="h-full">
      <ReaderShell
        {...shell}
        state={shellState(tab)}
        active={active}
        outline={outline}
        signatures={signatures}
        latest={() => latestInfo(tab.tab)}
        onClose={() => onClose(tab.tab)}
        onRetry={() => onRetry(tab.tab)}
        onUnlock={(password, remember) => onUnlock(tab.tab, password, remember)}
        onForgetPassword={() => onForgetPassword(tab.tab)}
      />
    </div>
  );
}
