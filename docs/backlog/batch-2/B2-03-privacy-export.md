---
title: "[B2-03] 隱私匯出：清除中繼資料後另存"
labels: task,batch-2,area:worker,area:app,area:ui,agent:core,agent:frontend,needs-security-review
---

- **需求 ID**：規格 §3「中繼資料清除」
- **負責角色**：核心 agent（清除）＋前端 agent（對話框）
- **相依**：B2-02（寫出與另存對話框）；QA-01 語料

## 目標
「⋯」→「隱私匯出…」產生一份副本，抹除作者、軟體版本、作業系統、GPS、修改時間與文件識別碼等數位足跡；原始檔案不受影響。

## 範圍
- worker 在記憶體中的文件副本上清除：
  - 文件資訊字典（trailer `/Info`：作者、標題、主旨、關鍵字、建立者、產生器、日期）；
  - 文件層與各物件上的 XMP 中繼資料串流（`/Metadata`）；
  - `/PieceInfo`（應用程式私有資料）、頁面縮圖（`/Thumb`）；
  - trailer 的 `/ID` 重新以亂數產生（原本的 `/ID` 可能含有建立時的時間與路徑雜湊）；
  - 本 app 之後寫入的文件識別碼（ADR 0010）。
- 以完整重寫輸出（不是增量更新），並移除未使用的物件（`garbage`），避免舊內容留在檔案中。
- 匯出前的對話框列出會清除的項目；只能另存新檔，不能覆寫原檔。
- 已簽章的文件：說明清除後簽章會失效。
- 語料：新增含 `/Info`、文件與頁面 XMP、`/PieceInfo`、縮圖的樣本（`tests/corpus/generate.py`）。

## 不做什麼
- 內嵌圖片的 EXIF（JPEG 的 APP1 區段）：需要重新編碼圖片串流，另開卡（與規格 §5 的 EXIF 清除一起）。
- 批次處理（ADR 0005，另開卡）。

## 可動的模組
- `crates/pdf_worker/`、`crates/ipc_contract/`、`src-tauri/src/`
- `src/features/`（匯出對話框）、`src/i18n/zh-TW.ts`
- `tests/corpus/`、`docs/architecture/`（新文件：清除了什麼、沒清除什麼）

## 驗收情境
- 假設匯出含完整中繼資料的語料，當以 MuPDF 讀取輸出檔，則 `/Info`、所有 `/Metadata`、`/PieceInfo`、`/Thumb` 都不存在，`/ID` 與原檔不同，頁面內容與原檔渲染結果相同。
- 原檔的位元組完全不變。
- 輸出檔中找不到原本作者名稱等字串（對整個檔案做位元組搜尋，涵蓋壓縮前後）。

## 必跑測試
- worker 測試：上述每一項清除；渲染結果不變。
- E2E：匯出並重新開啟。

## 資安限制
- 清除在 worker 中進行；主行程不解析 PDF。
- 文件中說明「沒有清除」的部分（例如頁面內容中的文字、圖片的 EXIF），避免使用者誤以為已完全匿名。
