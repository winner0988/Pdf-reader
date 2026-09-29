---
title: "[B2-08] 註解：手繪線條與印章"
labels: task,batch-2,area:worker,area:ipc,area:ui,agent:core,agent:frontend,needs-security-review
---

- **需求 ID**：規格 §6「註解標記：手繪線條、自訂印章」；規格 §5「圖片」（自訂印章的圖片）
- **負責角色**：前端 agent＋核心 agent
- **相依**：B2-07

## 目標
在頁面上手繪線條，或蓋上內建或自訂的印章；存成標準的 PDF 註解（`Ink`、`Stamp`）。

## 範圍
- 手繪：畫筆工具（顏色、粗細），以滑鼠或觸控筆繪製；縮放、旋轉時座標正確。
- 印章：內建幾種文字印章（例如「已核准」「草稿」）；自訂印章可以選本機圖片（PNG／JPG）。
  - 圖片由主行程的開啟對話框選取、以唯讀 handle 交給 worker；**寫入前清除 EXIF**（規格 §5：GPS、相機型號、拍攝時間），並在本機重新編碼。
- 選取、移動、縮放、刪除；復原／重做。
- 權限（MVP-19）：作者禁止註解時停用。

## 不做什麼
- 圖片以外的附件；簽名圖章的簽章功能（B2-11 之後）。

## 可動的模組
- `src/features/viewer/`、`src/features/shell/`、`src/i18n/zh-TW.ts`
- `crates/ipc_contract/`、`crates/pdf_worker/`、`src-tauri/src/`

## 驗收情境
- 假設在第 1 頁畫一條線並存檔，當重新開啟，則有 `Ink` 註解，路徑點與畫的位置相符（考慮縮放與旋轉）。
- 假設以含 GPS 的 JPEG 作為印章，則存檔後的 PDF 中找不到該 GPS 資料與相機型號。

## 必跑測試
- worker 測試：`Ink` 路徑、`Stamp` 外觀、圖片 EXIF 已清除。
- 前端：繪製座標轉換（縮放、旋轉）。
- 語料：新增含 EXIF（GPS、相機型號）的測試圖片，由腳本產生。

## 資安限制
- 圖片也是不受信任的輸入：在 worker（沙盒）中解碼，有尺寸與像素上限。
- 路徑只在主行程。
