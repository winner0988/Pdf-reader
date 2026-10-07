import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { OcrApi } from "@/features/ocr/api";
import { OcrSection } from "@/features/ocr/OcrSection";
import { DEFAULT_SETTINGS, type Settings, type SettingsApi } from "@/features/settings/api";
import { SettingsProvider } from "@/features/settings/SettingsProvider";
import { strings } from "@/i18n/zh-TW";
import type { OcrLanguages } from "@/ipc/generated/contract";

const t = strings.ocr.settings;

const bundled: OcrLanguages = {
  languages: [
    { code: "chi_tra", bundled: true, bytes: 2_366_642n },
    { code: "eng", bundled: true, bytes: 4_113_088n },
  ],
  automatic: "chi_tra",
};
const withGerman: OcrLanguages = {
  languages: [...bundled.languages, { code: "deu", bundled: false, bytes: 3_145_728n }],
  automatic: "chi_tra",
};

function setup(saved: Settings = DEFAULT_SETTINGS, languages: OcrLanguages = bundled) {
  const settingsApi = {
    get: vi.fn<SettingsApi["get"]>(() => Promise.resolve(saved)),
    set: vi.fn<SettingsApi["set"]>(() => Promise.resolve()),
  } satisfies SettingsApi;
  const api = {
    languages: vi.fn<OcrApi["languages"]>(() => Promise.resolve(languages)),
    importLanguage: vi.fn<OcrApi["importLanguage"]>(() => Promise.resolve({ kind: "cancelled" })),
    removeLanguage: vi.fn<OcrApi["removeLanguage"]>(() => Promise.resolve(bundled)),
    start: vi.fn<OcrApi["start"]>(() => Promise.resolve()),
    stop: vi.fn<OcrApi["stop"]>(() => Promise.resolve()),
    focus: vi.fn<OcrApi["focus"]>(() => Promise.resolve()),
  } satisfies OcrApi;
  render(
    <SettingsProvider api={settingsApi}>
      <OcrSection api={api} />
    </SettingsProvider>,
  );
  return { api, settingsApi, user: userEvent.setup() };
}

const languageSelect = () => screen.getByRole("combobox", { name: t.language });

describe("the settings of recognising text (B2-10)", () => {
  it("starts by itself unless that is turned off, and saves the choice", async () => {
    const { settingsApi, user } = setup();
    const box = screen.getByRole("checkbox", { name: new RegExp(t.auto) });
    expect(box).toBeChecked();
    await user.click(box);
    expect(box).not.toBeChecked();
    expect(settingsApi.set).toHaveBeenLastCalledWith({ ...DEFAULT_SETTINGS, ocrAuto: false });
  });

  it("lists the languages, with the one the app chooses first, and saves the one picked", async () => {
    const { settingsApi, user } = setup();
    await waitFor(() => expect(within(languageSelect()).getAllByRole("option")).toHaveLength(3));
    const options = within(languageSelect()).getAllByRole("option");
    expect(options.map((option) => option.textContent)).toEqual([
      t.automatic("繁體中文（chi_tra）"),
      "繁體中文（chi_tra）",
      "English（eng）",
    ]);
    await user.selectOptions(languageSelect(), "eng");
    expect(settingsApi.set).toHaveBeenLastCalledWith({ ...DEFAULT_SETTINGS, ocrLanguage: "eng" });
    await user.selectOptions(languageSelect(), "");
    expect(settingsApi.set).toHaveBeenLastCalledWith({ ...DEFAULT_SETTINGS, ocrLanguage: null });
  });

  it("shows the app's choice for a language that is not installed any more", async () => {
    setup({ ...DEFAULT_SETTINGS, ocrLanguage: "fra" });
    await waitFor(() => expect(within(languageSelect()).getAllByRole("option")).toHaveLength(3));
    expect(languageSelect()).toHaveValue("");
  });

  it("imports a language: the main process asks for the file, and the page hears how it went", async () => {
    const { api, user } = setup();
    api.importLanguage.mockResolvedValueOnce({ kind: "imported", languages: withGerman });
    await user.click(screen.getByRole("button", { name: t.importButton }));
    expect(api.importLanguage).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(t.imported)).toBeInTheDocument();
    const list = screen.getByRole("list", { name: t.importedLanguages });
    expect(within(list).getByText(/Deutsch（deu）.*3\.0 MB/)).toBeInTheDocument();
    expect(within(languageSelect()).getByRole("option", { name: "Deutsch（deu）" })).toBeInTheDocument();
  });

  it("says why a file was refused, and does nothing when the dialog is closed", async () => {
    const { api, user } = setup();
    for (const reason of ["notLanguageData", "tooLarge", "nameTaken", "badName", "tooMany", "unreadable"] as const) {
      api.importLanguage.mockResolvedValueOnce({ kind: "refused", reason });
      await user.click(screen.getByRole("button", { name: t.importButton }));
      expect(await screen.findByText(t.refused[reason])).toBeInTheDocument();
    }
    api.importLanguage.mockResolvedValueOnce({ kind: "cancelled" });
    await user.click(screen.getByRole("button", { name: t.importButton }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(""));
    api.importLanguage.mockRejectedValueOnce({ code: "internal", message: "" });
    await user.click(screen.getByRole("button", { name: t.importButton }));
    expect(await screen.findByText(t.failed)).toBeInTheDocument();
  });

  it("removes an imported language, and leaves it to the app if that was the one chosen", async () => {
    const { api, settingsApi, user } = setup({ ...DEFAULT_SETTINGS, ocrLanguage: "deu" }, withGerman);
    const remove = await screen.findByRole("button", { name: t.remove("Deutsch（deu）") });
    await user.click(remove);
    expect(api.removeLanguage).toHaveBeenCalledWith("deu");
    expect(await screen.findByText(t.removed)).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: t.importedLanguages })).toBeNull();
    expect(settingsApi.set).toHaveBeenLastCalledWith({ ...DEFAULT_SETTINGS, ocrLanguage: null });
    // Those that came with the app have no such button.
    expect(screen.queryAllByRole("button", { name: /^移除/ })).toHaveLength(0);
  });
});
