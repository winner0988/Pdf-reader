// The update check (#64, ADR 0009): the app's one network request, sent by the main process to a
// fixed GitHub address only when the user presses the button. Nothing from here goes into it,
// and the releases page it offers is a fixed address too, opened after the usual confirmation.

import { invoke } from "@tauri-apps/api/core";

import type { LinkPreview, UpdateCheck } from "@/ipc/generated/contract";

export type { UpdateCheck };

export type UpdatesApi = {
  /** One request to GitHub; rejects when there is no usable answer (offline, for example). */
  check(): Promise<UpdateCheck>;
  /** What the link confirmation shows about the releases page. */
  describeReleasesPage(): Promise<LinkPreview>;
  /** Opens the releases page in the browser, once the user confirmed it. */
  openReleasesPage(): Promise<void>;
};

export const tauriUpdatesApi: UpdatesApi = {
  check: () => invoke<UpdateCheck>("check_for_updates"),
  describeReleasesPage: () => invoke<LinkPreview>("describe_releases_page"),
  openReleasesPage: () => invoke<void>("open_releases_page"),
};
