import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { SEARCH_DELAY_MS, useSearch, type SearchApi } from "@/features/search/useSearch";
import { SearchBar } from "@/features/shell/SearchBar";
import { strings } from "@/i18n/zh-TW";
import type { IpcError, Quad, SearchArgs, SearchEvent } from "@/ipc/generated/contract";

const t = strings.search;

type Call = {
  args: SearchArgs;
  emit: (event: SearchEvent) => void;
  resolve: () => void;
  reject: (error: IpcError) => void;
};

/** Records every search; the test plays the main process. */
function fakeSearchApi() {
  const calls: Call[] = [];
  const api = {
    search: vi.fn(
      (args: SearchArgs, onEvent: (event: SearchEvent) => void) =>
        new Promise<void>((resolve, reject) => {
          calls.push({ args, emit: (event) => act(() => onEvent(event)), resolve, reject });
        }),
    ),
    cancel: vi.fn(() => Promise.resolve()),
  } satisfies SearchApi;
  return { api, calls };
}

const quad: Quad = { ul: { x: 0, y: 0 }, ur: { x: 1, y: 0 }, ll: { x: 0, y: 1 }, lr: { x: 1, y: 1 } };
const hits = (pageIndex: number, count: number): SearchEvent => ({
  kind: "hits",
  pageIndex,
  hits: Array.from({ length: count }, () => ({ quads: [quad] })),
});
const done = (totalHits: number, more: Partial<Extract<SearchEvent, { kind: "done" }>> = {}): SearchEvent => ({
  kind: "done",
  totalHits,
  truncated: false,
  noTextLayer: false,
  ...more,
});

function Harness({ api }: { api: SearchApi }) {
  const [open, setOpen] = useState(true);
  const search = useSearch({ api, doc: 3, active: open });
  if (!open) return null;
  return (
    <SearchBar
      search={search}
      pageCount={10}
      onClose={() => {
        search.clear();
        setOpen(false);
      }}
    />
  );
}

/**
 * Drives the bar with fireEvent: user-event waits on timers, which are fake here so that the
 * typing pause can be measured exactly.
 */
function setup() {
  const fake = fakeSearchApi();
  render(<Harness api={fake.api} />);
  const field = screen.getByRole<HTMLInputElement>("textbox", { name: t.placeholder });
  const user = {
    type: (text: string) => fireEvent.change(field, { target: { value: field.value + text } }),
    press: (key: string, shiftKey = false) => fireEvent.keyDown(field, { key, shiftKey }),
    click: (name: string) => fireEvent.click(screen.getByRole("button", { name })),
  };
  return { ...fake, user, field };
}

const status = () => document.querySelector("[data-status]")?.textContent ?? "";
const wait = (ms: number) => act(() => vi.advanceTimersByTime(ms));

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("search bar", () => {
  it("searches once typing pauses, shows progress, then the hit count", () => {
    const { api, calls, user } = setup();

    user.type("needle");
    wait(SEARCH_DELAY_MS - 1);
    expect(api.search).not.toHaveBeenCalled();
    wait(1);
    expect(api.search).toHaveBeenCalledTimes(1);
    expect(calls[0]!.args).toEqual({ request: expect.any(Number), doc: 3, query: "needle", caseSensitive: false });

    calls[0]!.emit({ kind: "progress", pagesSearched: 4 });
    expect(status()).toBe(t.progress(4, 10));
    calls[0]!.emit(hits(2, 2));
    calls[0]!.emit(done(2));
    expect(status()).toBe(t.count(1, 2));
    expect(screen.getByRole("status")).toHaveTextContent(t.count(1, 2));
  });

  it("Enter searches right away, then steps through the hits; Shift+Enter goes back", () => {
    const { api, calls, user } = setup();

    user.type("needle");
    user.press("Enter");
    expect(api.search).toHaveBeenCalledTimes(1);
    calls[0]!.emit(hits(0, 1));
    calls[0]!.emit(hits(5, 2));
    calls[0]!.emit(done(3));
    expect(status()).toBe(t.count(1, 3));

    user.press("Enter");
    expect(status()).toBe(t.count(2, 3));
    user.press("Enter", true);
    user.press("Enter", true);
    expect(status()).toBe(t.count(3, 3));
    user.click(t.next);
    expect(status()).toBe(t.count(1, 3));
    // Already searched: the pause after typing does not search again.
    wait(SEARCH_DELAY_MS);
    expect(api.search).toHaveBeenCalledTimes(1);
  });

  it("editing the query cancels the running search, whose late results are ignored", () => {
    const { api, calls, user } = setup();

    user.type("need");
    wait(SEARCH_DELAY_MS);
    user.type("le");
    expect(api.cancel).toHaveBeenCalledWith(calls[0]!.args.request);
    calls[0]!.emit(hits(0, 5));
    calls[0]!.emit(done(5));
    expect(status()).toBe("");

    wait(SEARCH_DELAY_MS);
    expect(calls[1]!.args.query).toBe("needle");
    expect(calls[1]!.args.request).not.toBe(calls[0]!.args.request);
    calls[1]!.emit(hits(1, 1));
    calls[1]!.emit(done(1));
    expect(status()).toBe(t.count(1, 1));
  });

  it("Escape cancels the running search and closes the bar", () => {
    const { api, calls, user } = setup();

    user.type("needle");
    user.press("Enter");
    user.press("Escape");
    expect(api.cancel).toHaveBeenCalledWith(calls[0]!.args.request);
    expect(screen.queryByRole("search")).not.toBeInTheDocument();
  });

  it("says when nothing was found, when there is no text layer, and when results were cut off", () => {
    const { calls, user } = setup();

    user.type(" needle ");
    user.press("Enter");
    calls[0]!.emit(done(0));
    expect(status()).toBe(t.noResults("needle"));

    user.click(t.caseSensitive);
    calls[1]!.emit(done(0, { noTextLayer: true }));
    expect(status()).toBe(t.noTextLayer);

    user.click(t.caseSensitive);
    calls[2]!.emit(hits(0, 3));
    calls[2]!.emit(done(10000, { truncated: true }));
    expect(status()).toBe("結果超過 10,000 筆，只顯示前 10,000 筆");
  });

  it("Aa searches again at once, case-sensitively", () => {
    const { calls, user } = setup();

    user.type("Needle");
    user.click(t.caseSensitive);
    expect(screen.getByRole("button", { name: t.caseSensitive })).toHaveAttribute("aria-pressed", "true");
    expect(calls).toHaveLength(1);
    expect(calls[0]!.args).toMatchObject({ query: "Needle", caseSensitive: true });
  });

  it("a failed search says so; a cancelled or closed-document one says nothing", async () => {
    const { calls, user } = setup();

    user.type("a");
    user.press("Enter");
    await act(async () => calls[0]!.reject({ code: "unknownDocument", message: "" }));
    expect(status()).toBe(t.progress(0, 10));

    user.type("b");
    user.press("Enter");
    await act(async () => calls[1]!.reject({ code: "workerCrashed", message: "" }));
    expect(status()).toBe(t.failed);
    // Enter tries a failed search again.
    user.press("Enter");
    expect(calls).toHaveLength(3);
  });

  it("does not search what an input method is still composing", () => {
    const { api, field } = setup();

    fireEvent.compositionStart(field);
    fireEvent.change(field, { target: { value: "ㄓㄨㄥ" } });
    wait(SEARCH_DELAY_MS * 2);
    fireEvent.keyDown(field, { key: "Enter", isComposing: true });
    expect(api.search).not.toHaveBeenCalled();

    fireEvent.change(field, { target: { value: "中" } });
    fireEvent.compositionEnd(field);
    wait(SEARCH_DELAY_MS);
    expect(api.search).toHaveBeenCalledTimes(1);
    expect(api.search.mock.calls[0]![0].query).toBe("中");
  });

  it("refuses queries over the byte limit", () => {
    const { field } = setup();

    fireEvent.change(field, { target: { value: "搜".repeat(341) } }); // 1,023 bytes
    expect(field).toHaveValue("搜".repeat(341));
    fireEvent.change(field, { target: { value: "搜".repeat(342) } }); // 1,026 bytes
    expect(field).toHaveValue("搜".repeat(341));
  });
});
