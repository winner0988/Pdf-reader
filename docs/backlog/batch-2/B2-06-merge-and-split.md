---
title: "[B2-06] 合併與拆分 PDF"
labels: task,batch-2,area:worker,area:app,area:ipc,area:ui,agent:core,agent:frontend,needs-security-review
---

- **需求 ID**：規格 §6「拆分／合併 PDF」
- **負責角色**：核心 agent＋前端 agent
- **相依**：B2-05

## 目標
把其他 PDF 的頁面插入目前的文件（合併），或把頁面範圍另存成新檔（拆分）。

## 範圍
- **合併**：「插入其他檔案的頁面…」→ 主行程的開啟對話框 → 選擇插入位置。
  - 來源檔同樣是不受信任的 PDF：由主行程把它的**唯讀 handle** 交給目前文件的 worker，在同一個沙盒中開啟並複製頁面（MuPDF 的 graft）。
  - 來源檔加密時先詢問密碼（MVP-16 的流程）。
  - 來源檔的主動內容（JavaScript、`/OpenAction` 等）不複製進來；掃描結果（MVP-11）合併到目前文件的警示。
- **拆分**：選取頁面或輸入範圍 → 另存成一個新檔；或「每 N 頁一個檔案」存到資料夾。
- 上限：一次合併的頁數與檔案數。

## 不做什麼
- 同時開多個來源檔的拼版介面；書籤的合併以 MuPDF 的預設行為為準（寫進文件）。

## 可動的模組
- `crates/pdf_worker/`、`crates/worker_host/`、`crates/ipc_contract/`、`src-tauri/src/`
- `src/features/thumbnails/`、`src/features/shell/`、`src/i18n/zh-TW.ts`

## 驗收情境
- 假設在 10 頁文件的第 3 頁後插入 `benign/single-page.pdf`，當存檔並重新開啟，則有 11 頁，第 4 頁是插入的頁面。
- 假設插入含 JavaScript 的語料（`malicious/`），則合併後的文件不含該腳本，警示橫幅列出偵測到的內容。
- 假設拆分第 2–4 頁，則新檔有 3 頁，內容與原頁相同。

## 必跑測試
- worker 測試：graft、主動內容不被複製、上限。
- E2E：合併與拆分各一次（對話框以 UI Automation 回答）。

## 資安限制
- worker 只拿到來源檔的唯讀 handle，拿不到路徑；每份來源檔都在目前文件的沙盒 worker 中開啟。
- 修改 worker 的 handle 傳遞方式需要 `needs-security-review`。
