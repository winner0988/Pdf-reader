// Full-screen window commands (B4-03).

import { invoke } from "@tauri-apps/api/core";

export type WindowApi = {
  /** Sets whether this window is in full-screen mode, and returns the resulting state. */
  setFullscreen(on: boolean): Promise<boolean>;
};

export const tauriWindowApi: WindowApi = {
  setFullscreen: (on: boolean) => invoke<boolean>("set_fullscreen", { on }),
};
