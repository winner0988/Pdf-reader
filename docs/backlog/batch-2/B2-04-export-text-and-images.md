---
title: "[B2-04] 匯出純文字與頁面圖片"
labels: task,batch-2,area:app,area:worker,area:ui,agent:core,agent:frontend,needs-security-review
---

- **需求 ID**：規格 §7「格式匯出」（純文字、圖片；Word 另開 POC）
- **負責角色**：核心 agent＋前端 agent
- **相依**：無（需要主行程的另存對話框：若 B2-02 尚未完成，本卡先做對話框模組，B2-02 沿用）

## 目標
把文件的文字匯出為 `.txt`，把頁面匯出為 PNG 或 JPG（逐頁、可選範圍與解析度），不經過任何雲端服務。

## 範圍
- **文字**：依頁序輸出 UTF-8，頁與頁之間以換頁分隔；沿用 `page_text` 的清理規則（MVP-15：不含控制字元、雙向文字控制與零寬字元）。
- **圖片**：worker 渲染（沿用 `render`，不套用檢視的旋轉），由 worker 以 MuPDF 編碼成 PNG／JPG，不新增影像依賴；可選 72／150／300 dpi 與頁面範圍（沿用列印的範圍語法）。
- 檔名由主行程產生：`<原檔名>-p<頁碼>.png`；目的地資料夾以系統的資料夾對話框選擇（不加入最近使用的項目）。已存在的檔案要先確認才覆寫。
- 進度與取消；一次最多 1,000 頁。
- **權限**（MVP-19）：作者禁止複製時，文字與圖片匯出都停用並說明原因（比照 Acrobat 的「內容複製」）。
- 沒有文字層的頁面：匯出的文字檔中標示「（此頁沒有文字層）」。

## 不做什麼
- Word（.docx）匯出：需要 POC 與 ADR，另開卡。
- OCR（B2-10）。

## 可動的模組
- `crates/pdf_worker/`、`crates/ipc_contract/`、`src-tauri/src/`（對話框、寫檔）
- `src/features/`（匯出對話框）、`src/i18n/zh-TW.ts`

## 驗收情境
- 假設匯出 `benign/mixed-text-zh-en.pdf` 的文字，則 `.txt` 含中英文兩行，編碼為 UTF-8，沒有控制字元。
- 假設以 150 dpi 匯出 Letter 頁面為 PNG，則圖片寬 1275 px。
- 假設文件禁止複製（`benign/restricted-no-copy-no-print.pdf`），則匯出選項停用並說明原因。
- 假設目的地已有同名檔案，則先詢問。

## 必跑測試
- worker 測試：文字內容與清理、PNG／JPG 的尺寸與格式標頭。
- E2E：匯出後讀回檔案內容。

## 資安限制
- 路徑只在主行程；worker 不寫任何檔案（編碼後的位元組回傳給主行程，受 frame 大小上限限制）。
- 單張圖片大小受 `LIMITS.maxRasterPixels` 限制。
