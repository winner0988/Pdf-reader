import { invoke } from "@tauri-apps/api/core";

import type { FontId, FontImport, FontInfo } from "@/ipc/generated/contract";

export type FontsApi = {
  getFonts(): Promise<FontInfo[]>;
  importFont(): Promise<FontImport>;
  removeFont(id: FontId): Promise<FontInfo[]>;
  removeAllFonts(): Promise<FontInfo[]>;
};

export const tauriFontsApi: FontsApi = {
  getFonts: () => invoke<FontInfo[]>("get_fonts"),
  importFont: () => invoke<FontImport>("import_font"),
  removeFont: (id) => invoke<FontInfo[]>("remove_font", { args: { id } }),
  removeAllFonts: () => invoke<FontInfo[]>("remove_all_fonts"),
};
