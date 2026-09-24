// "Set as default PDF app" (REL-03). Windows 10 and 11 let only the user choose default apps, so
// the main process opens the page of Windows Settings for PDF Reader. The address is fixed in
// the main process; nothing from here reaches the system.

import { invoke } from "@tauri-apps/api/core";

export type SystemApi = {
  openDefaultAppsSettings(): Promise<void>;
};

export const tauriSystemApi: SystemApi = {
  openDefaultAppsSettings: () => invoke<void>("open_default_apps_settings"),
};
