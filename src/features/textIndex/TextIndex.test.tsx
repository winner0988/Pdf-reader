import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_SETTINGS } from "@/features/settings/api";
import { SettingsContext, type SettingsContextValue } from "@/features/settings/useSettings";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { findShortcut } from "@/features/shortcuts/registry";
import type { TextIndexApi } from "@/features/textIndex/api";
import { initialIndexSearch, reduceIndexSearch, type IndexResult } from "@/features/textIndex/model";
import { SearchAllDialog } from "@/features/textIndex/SearchAllDialog";
import { TextIndexHost } from "@/features/textIndex/TextIndexHost";
import { TextIndexSection } from "@/features/textIndex/TextIndexSection";
import { strings } from "@/i18n/zh-TW";
import type { IndexSearchEvent, Settings } from "@/ipc/generated/contract";

const t = strings.textIndex;

function result(entry: number, displayName: string, pageIndex = 2): IndexResult {
  return {
    kind: "found",
    entry,
    displayName,
    totalHits: 3,
    pages: [{ pageIndex, hits: 2, snippet: { before: "the ", matched: "Harbour Bridge", after: " project" } }],
    pagesWithHits: 2,
    partial: false,
  };
}

function fakeApi(events: IndexSearchEvent[] = [result(1, "budget.pdf"), { kind: "done", documents: 4, found: 1 }]) {
  return {
    search: vi.fn<TextIndexApi["search"]>((_args, onEvent) => {
      events.forEach(onEvent);
      return Promise.resolve();
    }),
    cancel: vi.fn<TextIndexApi["cancel"]>(() => Promise.resolve()),
    open: vi.fn<TextIndexApi["open"]>(() => Promise.resolve(7)),
    status: vi.fn<TextIndexApi["status"]>(() => Promise.resolve({ documents: 12, bytes: 3 * 1024 * 1024 })),
    clear: vi.fn<TextIndexApi["clear"]>(() => Promise.resolve()),
  } satisfies TextIndexApi;
}

function settingsValue(change: Partial<Settings> = {}): SettingsContextValue {
  return {
    settings: { ...DEFAULT_SETTINGS, textIndex: true, ...change },
    update: vi.fn(),
    saveFailed: false,
  };
}

function renderDialog(api: TextIndexApi, settings = settingsValue()) {
  const onOpened = vi.fn();
  const onOpenChange = vi.fn();
  const user = userEvent.setup();
  render(
    <SettingsContext.Provider value={settings}>
      <SearchAllDialog open onOpenChange={onOpenChange} api={api} onOpened={onOpened} delayMs={0} />
    </SettingsContext.Provider>,
  );
  return { user, onOpened, onOpenChange };
}

describe("the results of a search of the text index (B3-21)", () => {
  it("collects the documents found and ignores a replaced search", () => {
    let state = reduceIndexSearch(initialIndexSearch, { type: "start", request: 1, query: "harbour", caseSensitive: false });
    state = reduceIndexSearch(state, { type: "event", request: 1, event: result(1, "a.pdf") });
    state = reduceIndexSearch(state, { type: "event", request: 9, event: result(2, "old.pdf") });
    state = reduceIndexSearch(state, { type: "event", request: 1, event: { kind: "done", documents: 5, found: 1 } });
    expect(state).toMatchObject({ kind: "done", documents: 5 });
    expect(state.kind !== "idle" && state.results.map((found) => found.displayName)).toEqual(["a.pdf"]);

    state = reduceIndexSearch(state, { type: "gone", entry: 1 });
    expect(state.kind !== "idle" && state.results).toEqual([]);
    expect(reduceIndexSearch(state, { type: "clear" })).toEqual(initialIndexSearch);
  });
});

describe("searching every document (B3-21)", () => {
  it("lists the documents found with their pages and the words around each hit", async () => {
    const api = fakeApi();
    const { user } = renderDialog(api);
    await user.type(screen.getByRole("textbox", { name: t.label }), "harbour bridge");
    const list = await screen.findByRole("list", { name: t.results });
    expect(within(list).getByText("budget.pdf")).toBeInTheDocument();
    expect(within(list).getByText(t.hits(3, 2))).toBeInTheDocument();
    expect(within(list).getByText("Harbour Bridge").tagName).toBe("MARK");
    expect(within(list).getByText(t.morePages(1))).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(t.found(1, 4));
    expect(api.search).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "harbour bridge", caseSensitive: false }),
      expect.any(Function),
    );
  });

  it("opens the document of a page in a new tab at that page", async () => {
    const api = fakeApi();
    const { user, onOpened } = renderDialog(api);
    await user.type(screen.getByRole("textbox", { name: t.label }), "harbour");
    await user.click(await screen.findByRole("button", { name: new RegExp(t.page(3)) }));
    expect(api.open).toHaveBeenCalledWith(1);
    await waitFor(() => expect(onOpened).toHaveBeenCalledWith(7, 3));
  });

  it("takes out a document whose file is gone and says so", async () => {
    const api = fakeApi();
    api.open.mockRejectedValue({ code: "unreadable", message: "gone" });
    const { user, onOpened } = renderDialog(api);
    await user.type(screen.getByRole("textbox", { name: t.label }), "harbour");
    await user.click(await screen.findByRole("button", { name: new RegExp(t.page(3)) }));
    expect(await screen.findByRole("status")).toHaveTextContent(t.gone("budget.pdf"));
    expect(screen.queryByRole("list", { name: t.results })).not.toBeInTheDocument();
    expect(onOpened).not.toHaveBeenCalled();
  });

  it("says when nothing is found, and when the index has no documents yet", async () => {
    const api = fakeApi([{ kind: "done", documents: 3, found: 0 }]);
    const { user } = renderDialog(api);
    await user.type(screen.getByRole("textbox", { name: t.label }), "tunnel");
    expect(await screen.findByText(t.noResults("tunnel"))).toBeInTheDocument();

    api.search.mockImplementation((_args, onEvent) => {
      onEvent({ kind: "done", documents: 0, found: 0 });
      return Promise.resolve();
    });
    await user.type(screen.getByRole("textbox", { name: t.label }), "s");
    expect(await screen.findByText(t.empty)).toBeInTheDocument();
  });

  it("searches again with the case toggled", async () => {
    const api = fakeApi();
    const { user } = renderDialog(api);
    await user.type(screen.getByRole("textbox", { name: t.label }), "Harbour");
    await screen.findByRole("list", { name: t.results });
    await user.click(screen.getByRole("button", { name: t.caseSensitive }));
    await waitFor(() =>
      expect(api.search).toHaveBeenLastCalledWith(
        expect.objectContaining({ query: "Harbour", caseSensitive: true }),
        expect.any(Function),
      ),
    );
  });

  it("cancels a running search when the query changes", async () => {
    const api = fakeApi();
    api.search.mockImplementation(() => new Promise(() => {}));
    const { user } = renderDialog(api);
    await user.type(screen.getByRole("textbox", { name: t.label }), "a");
    await waitFor(() => expect(api.search).toHaveBeenCalledTimes(1));
    const request = api.search.mock.calls[0]![0].request;
    await user.type(screen.getByRole("textbox", { name: t.label }), "b");
    expect(api.cancel).toHaveBeenCalledWith(request);
  });

  it("searches nothing while the index is off, and says where to turn it on", async () => {
    const api = fakeApi();
    renderDialog(api, settingsValue({ textIndex: false }));
    expect(screen.getByText(t.off)).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: t.label })).toBeDisabled();

    // Off with the recent files list too, whatever its own setting says.
    renderDialog(api, settingsValue({ textIndex: true, recordRecentFiles: false }));
    expect(screen.getAllByText(t.off)).toHaveLength(2);
    expect(api.search).not.toHaveBeenCalled();
  });

  it("is Ctrl+Shift+F, and Ctrl+F is still the search of one document", () => {
    const keydown = (init: KeyboardEventInit) => new KeyboardEvent("keydown", { bubbles: true, ...init });
    expect(findShortcut(keydown({ key: "F", ctrlKey: true, shiftKey: true }))?.id).toBe("searchAll");
    expect(findShortcut(keydown({ key: "f", ctrlKey: true }))?.id).toBe("search");
  });
});

describe("where the search of every document is offered (B3-21)", () => {
  function renderShell(settings: SettingsContextValue) {
    const user = userEvent.setup();
    render(
      <SettingsContext.Provider value={settings}>
        <TextIndexHost api={fakeApi()} onOpened={vi.fn()}>
          <ReaderShell state={{ kind: "empty" }} onOpen={vi.fn()} loadingDelayMs={0} />
        </TextIndexHost>
      </SettingsContext.Provider>,
    );
    return user;
  }

  it("on the start screen while the index is on, and in the menu", async () => {
    const user = renderShell(settingsValue());
    await user.click(screen.getByRole("button", { name: strings.empty.searchAllButton }));
    expect(await screen.findByRole("dialog", { name: t.title })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: new RegExp(strings.menu.searchAll) }));
    expect(await screen.findByRole("dialog", { name: t.title })).toBeInTheDocument();
  });

  it("not on the start screen while the index is off", () => {
    renderShell(settingsValue({ textIndex: false }));
    expect(screen.queryByRole("button", { name: strings.empty.searchAllButton })).not.toBeInTheDocument();
  });
});

describe("the text index in the settings (B3-21)", () => {
  it("shows what it keeps, turns it on and off, and deletes it", async () => {
    const api = fakeApi();
    const settings = settingsValue({ textIndex: false });
    const user = userEvent.setup();
    render(
      <SettingsContext.Provider value={settings}>
        <TextIndexSection api={api} />
      </SettingsContext.Provider>,
    );
    const s = strings.settings.textIndex;
    expect(await screen.findByText(s.status(12, "3.0"))).toBeInTheDocument();
    await user.click(screen.getByRole("checkbox", { name: new RegExp(s.enable) }));
    expect(settings.update).toHaveBeenCalledWith({ textIndex: true });

    await user.click(screen.getByRole("button", { name: s.clear }));
    expect(api.clear).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole("status")).toHaveTextContent(s.cleared);
    await waitFor(() => expect(api.status).toHaveBeenCalledTimes(2));
  });

  it("cannot be turned on without the recent files list", async () => {
    const api = fakeApi();
    render(
      <SettingsContext.Provider value={settingsValue({ textIndex: true, recordRecentFiles: false })}>
        <TextIndexSection api={api} />
      </SettingsContext.Provider>,
    );
    const s = strings.settings.textIndex;
    const checkbox = screen.getByRole("checkbox", { name: new RegExp(s.enable) });
    expect(checkbox).toBeDisabled();
    expect(checkbox).not.toBeChecked();
    expect(screen.getByText(s.needsRecent)).toBeInTheDocument();
    await act(async () => {});
  });
});
