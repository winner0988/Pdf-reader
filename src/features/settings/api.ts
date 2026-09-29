// The user's settings (B2-12, docs/architecture/local-data.md). The main process keeps them in
// `settings.json` in the app's local data folder; the page only gets and replaces the whole set.

import { invoke } from "@tauri-apps/api/core";

import type { Settings } from "@/ipc/generated/contract";

export type { Settings };

export type SettingsApi = {
  get(): Promise<Settings>;
  /** Replaces the whole set. It applies at once; a rejection only means it was not saved. */
  set(settings: Settings): Promise<void>;
};

export const tauriSettingsApi: SettingsApi = {
  get: () => invoke<Settings>("get_settings"),
  set: (settings) => invoke<void>("set_settings", { settings }),
};

export const DEFAULT_SETTINGS: Settings = { theme: "system", recordRecentFiles: true };
