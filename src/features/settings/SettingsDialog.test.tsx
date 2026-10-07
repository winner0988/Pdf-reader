import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { RecentApi } from "@/features/recent/api";
import { DEFAULT_SETTINGS, type SettingsApi } from "@/features/settings/api";
import { SettingsProvider } from "@/features/settings/SettingsProvider";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import type { Settings } from "@/ipc/generated/contract";
import { mediaQuery } from "@/test/setup";

const t = strings.settings;

function fakeSettingsApi(saved: Settings = DEFAULT_SETTINGS) {
  return {
    get: vi.fn<SettingsApi["get"]>(() => Promise.resolve(saved)),
    set: vi.fn<SettingsApi["set"]>(() => Promise.resolve()),
  } satisfies SettingsApi;
}

function fakeRecentApi() {
  return {
    list: vi.fn<RecentApi["list"]>(() => Promise.resolve([])),
    open: vi.fn<RecentApi["open"]>(() => Promise.resolve()),
    remove: vi.fn<RecentApi["remove"]>(() => Promise.resolve([])),
    clear: vi.fn<RecentApi["clear"]>(() => Promise.resolve()),
    clearExclusions: vi.fn<RecentApi["clearExclusions"]>(() => Promise.resolve()),
    isRecorded: vi.fn<RecentApi["isRecorded"]>(() => Promise.resolve(true)),
    setRecorded: vi.fn<RecentApi["setRecorded"]>(() => Promise.resolve()),
  } satisfies RecentApi;
}

const openDocument: ShellState = { kind: "open", document: { ...demoDocument, doc: 5 } };

function renderWithSettings(settingsApi: SettingsApi, recentApi: RecentApi, state: ShellState = openDocument) {
  const user = userEvent.setup();
  render(
    <SettingsProvider api={settingsApi}>
      <ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} recentApi={recentApi} />
    </SettingsProvider>,
  );
  return { user };
}

async function openSettings(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  await user.click(await screen.findByRole("menuitem", { name: strings.menu.settings }));
  return screen.findByRole("dialog", { name: t.title });
}

describe("settings (B2-12)", () => {
  it("the saved theme applies once the settings arrive", async () => {
    mediaQuery.matches = false;
    const settingsApi = fakeSettingsApi({ ...DEFAULT_SETTINGS, theme: "dark" });
    renderWithSettings(settingsApi, fakeRecentApi());
    await waitFor(() => expect(document.documentElement).toHaveClass("dark"));
    expect(settingsApi.get).toHaveBeenCalledTimes(1);
  });

  it("choosing a theme applies it and saves the whole set", async () => {
    mediaQuery.matches = false;
    const settingsApi = fakeSettingsApi();
    const { user } = renderWithSettings(settingsApi, fakeRecentApi());
    await act(async () => {});
    const dialog = await openSettings(user);

    await user.click(within(dialog).getByRole("radio", { name: t.themes.dark }));
    expect(document.documentElement).toHaveClass("dark");
    expect(settingsApi.set).toHaveBeenLastCalledWith({ ...DEFAULT_SETTINGS, theme: "dark" });
    // The dialog shows the choice it saved.
    expect(within(dialog).getByRole("radio", { name: t.themes.dark })).toBeChecked();
  });

  it("turning off the recent files list saves it, and no file can be marked any more", async () => {
    const settingsApi = fakeSettingsApi();
    const { user } = renderWithSettings(settingsApi, fakeRecentApi());
    await act(async () => {});
    const dialog = await openSettings(user);

    await user.click(within(dialog).getByRole("checkbox", { name: new RegExp(t.record) }));
    expect(settingsApi.set).toHaveBeenLastCalledWith({ ...DEFAULT_SETTINGS, recordRecentFiles: false });
    await user.keyboard("{Escape}");

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await screen.findByRole("menuitem", { name: strings.menu.about });
    expect(screen.queryByRole("menuitemcheckbox", { name: strings.menu.dontRecord })).toBeNull();
  });

  it("clears the list and the files not to record, and says so", async () => {
    const recentApi = fakeRecentApi();
    const { user } = renderWithSettings(fakeSettingsApi(), recentApi);
    await act(async () => {});
    const dialog = await openSettings(user);

    await user.click(within(dialog).getByRole("button", { name: t.clearList }));
    expect(recentApi.clear).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(within(dialog).getByRole("status")).toHaveTextContent(t.listCleared));

    await user.click(within(dialog).getByRole("button", { name: t.clearExclusions }));
    expect(recentApi.clearExclusions).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(within(dialog).getByRole("status")).toHaveTextContent(t.exclusionsCleared));
  });

  it("says when a change could not be saved", async () => {
    const settingsApi = fakeSettingsApi();
    settingsApi.set.mockRejectedValue({ code: "unreadable", message: "" });
    const { user } = renderWithSettings(settingsApi, fakeRecentApi());
    await act(async () => {});
    const dialog = await openSettings(user);

    await user.click(within(dialog).getByRole("radio", { name: t.themes.light }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.saveFailed);
  });

  it("states what the app keeps on this computer", async () => {
    const { user } = renderWithSettings(fakeSettingsApi(), fakeRecentApi(), { kind: "empty" });
    const dialog = await openSettings(user);
    for (const item of t.dataItems) expect(within(dialog).getByText(item)).toBeInTheDocument();
    expect(within(dialog).getByText(t.dataLocation)).toBeInTheDocument();
  });
});
