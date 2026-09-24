import { describe, expect, it } from "vitest";

import { initialTabs, reduceTabs, shellState, type TabsAction, type TabsState } from "@/features/tabs/model";
import type { DocumentInfo, OpenEvent } from "@/ipc/generated/contract";

const info = (doc: number, displayName: string): DocumentInfo => ({
  doc,
  displayName,
  pages: [{ widthPt: 612, heightPt: 792 }],
  hasOutline: true,
  security: { findings: [], scanComplete: true },
});

const event = (e: OpenEvent): TabsAction => ({ type: "event", event: e });
const run = (actions: TabsAction[], state: TabsState = initialTabs) => actions.reduce(reduceTabs, state);
const names = (state: TabsState) => state.tabs.map((tab) => tab.displayName);

describe("tabs", () => {
  it("adds a tab per opening file and shows the newest", () => {
    const state = run([
      event({ kind: "opening", tab: 1, displayName: "a.pdf" }),
      event({ kind: "opening", tab: 2, displayName: "b.pdf" }),
    ]);
    expect(names(state)).toEqual(["a.pdf", "b.pdf"]);
    expect(state.active).toBe(2);
    expect(shellState(state.tabs[0]!)).toEqual({ kind: "loading", displayName: "a.pdf" });
  });

  it("fills a tab in when its file opened or failed, without switching to it", () => {
    const state = run([
      event({ kind: "opening", tab: 1, displayName: "a.pdf" }),
      event({ kind: "opening", tab: 2, displayName: "b.pdf" }),
      event({ kind: "opened", tab: 1, info: info(9, "a.pdf") }),
      event({ kind: "failed", tab: 2, displayName: "b.pdf", error: { code: "notPdf", message: "" } }),
    ]);
    expect(state.active).toBe(2);
    expect(state.tabs[0]!.content).toMatchObject({ kind: "open", hasOutline: true });
    expect(shellState(state.tabs[1]!)).toEqual({ kind: "error", code: "notPdf", displayName: "b.pdf" });
  });

  it("rebuilds the tabs of a reloaded page from the main process's snapshot", () => {
    const state = run([
      event({ kind: "opened", tab: 3, info: info(9, "a.pdf") }),
      event({ kind: "opened", tab: 5, info: info(10, "b.pdf") }),
    ]);
    expect(names(state)).toEqual(["a.pdf", "b.pdf"]);
    expect(state.active).toBe(3);
  });

  it("a retry keeps the tab where it is and does not switch to it", () => {
    const failedOne = run([
      event({ kind: "failed", tab: 1, displayName: "a.pdf", error: { code: "workerCrashed", message: "" } }),
      event({ kind: "opening", tab: 2, displayName: "b.pdf" }),
    ]);
    const state = reduceTabs(failedOne, event({ kind: "opening", tab: 1, displayName: "a.pdf" }));
    expect(names(state)).toEqual(["a.pdf", "b.pdf"]);
    expect(state.active).toBe(2);
    expect(state.tabs[0]!.content).toEqual({ kind: "loading" });
  });

  it("closing the shown tab shows the one on its right, else the one on its left", () => {
    const three = run([1, 2, 3].map((tab) => event({ kind: "opening", tab, displayName: `${tab}.pdf` })));
    const middle = run([{ type: "activate", tab: 2 }, { type: "closed", tab: 2 }], three);
    expect(middle.active).toBe(3);
    const last = run([{ type: "closed", tab: 3 }], middle);
    expect(last.active).toBe(1);
    expect(run([{ type: "closed", tab: 1 }], last).active).toBeNull();
    // Closing another tab keeps the shown one.
    expect(run([{ type: "activate", tab: 1 }, { type: "closed", tab: 3 }], three).active).toBe(1);
  });

  it("ignores events for a tab the user closed", () => {
    const state = run([
      event({ kind: "opening", tab: 1, displayName: "a.pdf" }),
      { type: "closed", tab: 1 },
      event({ kind: "opened", tab: 1, info: info(9, "a.pdf") }),
    ]);
    expect(state.tabs).toEqual([]);
    expect(state.active).toBeNull();
  });

  it("steps through the tabs in both directions, wrapping around", () => {
    const three = run([1, 2, 3].map((tab) => event({ kind: "opening", tab, displayName: `${tab}.pdf` })));
    expect(run([{ type: "step", by: 1 }], three).active).toBe(1);
    expect(run([{ type: "step", by: -1 }], three).active).toBe(2);
    expect(run([{ type: "step", by: 1 }], initialTabs)).toBe(initialTabs);
  });

  it("keeps notices about the tab limit and failed commands until dismissed", () => {
    const limited = run([event({ kind: "tabLimit", ignoredFiles: 4 })]);
    expect(limited.notice).toEqual({ kind: "tabLimit", ignoredFiles: 4 });
    expect(run([{ type: "failed", code: "internal" }]).notice).toEqual({ kind: "failed", code: "internal" });
    expect(run([{ type: "dismissNotice" }], limited).notice).toBeNull();
  });
});
