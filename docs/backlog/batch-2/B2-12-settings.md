---
title: "[B2-12] 設定頁與設定儲存"
labels: task,batch-2,area:app,area:ipc,area:ui,agent:core,agent:frontend,needs-security-review
---

- **需求 ID**：規格 §1（設定頁的「檢查更新」）、§2（OCR 語言包）、§3（信任與機敏管理）；README 下一批候選「設定儲存」
- **負責角色**：前端 agent＋核心 agent
- **相依**：無（之後的 #64 檢查更新、OCR 語言包、信任與機敏管理都放在這裡）

## 目標
一個集中的設定頁，設定跨次啟動保留；目前的外觀選擇（MVP-05）每次啟動都會回到「跟隨系統」。

## 範圍
- **儲存**：`settings.json` 放在 app 的本機資料資料夾（與 `recent.json` 相同，#73），由主行程讀寫。
  - 前端只能透過命令讀取與修改已知的設定項目，每一項都在 `ipc_contract` 中定義與驗證；未知欄位拒絕。
  - 檔案損毀或版本不對時使用預設值，下次變更時覆寫。
- **設定頁**：「⋯」→「設定」，用對話框或全頁；只用鍵盤可以操作。
  - 外觀：跟隨系統／淺色／深色（取代目前不保存的選擇）。
  - 最近開啟的檔案：是否記錄（關閉時等同不記錄任何檔案）、清除清單、清除「不記錄此檔案」的選擇。
  - 隱私：說明 app 在這台電腦上保存了哪些資料、在哪裡（`recent.json`、`settings.json`、WebView2 的資料），以及如何刪除。
- 「關於」維持在原本的位置。

## 不做什麼
- 檢查更新（#64，等 ADR 0009）、OCR 語言包（B2-10 之後）、信任與機敏管理（等 ADR 0002／0004／0010 的實作）。
- SQLite：目前的設定量不需要資料庫；若之後的全文索引需要，再另寫 ADR。

## 可動的模組
- `src-tauri/src/`（設定的讀寫與命令）、`crates/ipc_contract/`、`src-tauri/capabilities/`、`src-tauri/build.rs`、`scripts/ci/check-security-config.mjs`
- `src/features/`（新的 `settings/`）、`src/features/theme/`、`src/features/recent/`、`src/i18n/zh-TW.ts`
- `docs/architecture/`（新文件：app 在本機保存的資料）

## 驗收情境
- 假設選擇「深色」後重新啟動，則仍是深色。
- 假設關閉「記錄最近開啟的檔案」，當開啟檔案，則 `recent.json` 沒有新增項目。
- 假設 `settings.json` 被改成不是 JSON，則 app 以預設值啟動，不當機。

## 必跑測試
- Rust：讀寫、損毀的檔案、未知欄位、驗證。
- 前端：設定頁的鍵盤操作。
- E2E：設定跨次啟動保留（使用 `PDF_READER_DATA_DIR`，不碰使用者的設定）。

## 資安限制
- 前端不能寫入任意鍵值或檔案；只能改已定義的項目。
- 新命令都要在 capability 與 `check-security-config.mjs` 中說明用途。
