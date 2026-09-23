---
title: "[MVP-11] 主動內容與遠端引用：預設封鎖與偵測提示"
labels: task,mvp,area:worker,area:security,agent:core,needs-security-review
---

- **需求 ID**：MVP-R7、MVP-R8；ADR 0001、ADR 0002
- **負責角色**：核心 agent（QA／安全 agent 驗收）
- **相依**：MVP-04、MVP-05、QA-01

## 目標
PDF 內的主動內容與遠端引用一律不執行，並清楚告訴使用者擋下了什麼。

## 範圍
- 回歸測試：確認任何情況下都不會執行 PDF JavaScript（對應 MVP-03 的建置設定）。
- 開檔時由 worker 掃描並回報「安全發現」：
  - 文件層 JavaScript、`/OpenAction`、`/AA`（文件、頁面、註解、表單欄位各層）
  - `/Launch`、`/SubmitForm`、`/ImportData`、`/GoToR`、`/GoToE`
  - URL 或 UNC 型的檔案規格、XFA、`/RichMedia`
  - 嵌入檔案（僅提示，不開啟）
- UI：安全橫幅顯示摘要（例如「已封鎖 3 項主動內容：JavaScript、開檔動作、遠端引用」），可展開明細；**不提供「允許執行」按鈕**。
- 掃描有時間與數量上限，超過時標示「掃描未完成」。

## 不做什麼
- 表單腳本沙盒（ADR 0001 的例外）、信任例外與信任清單（ADR 0002）。
- 開啟或匯出嵌入檔案、從 PDF 移除這些內容（屬編輯功能）。

## 可動的模組
- `crates/pdf_worker/`（掃描器）
- `crates/ipc_contract/`（安全發現型別）
- `src/features/security-banner/`
- `docs/security/manual-checks.md`（新增）

## 驗收情境
- QA-01 惡意語料的每個檔案，開啟後都沒有執行腳本、沒有網路請求、沒有啟動外部程式、沒有存取語料以外的檔案。以 Process Monitor 手動驗證一次，步驟寫進 `docs/security/manual-checks.md`。
- 每個樣本的安全發現與 `manifest.json` 的預期清單一致（自動化測試）。
- 沒有主動內容的一般 PDF 不顯示橫幅。

## 必跑測試
- worker 單元測試：每個惡意樣本對應的安全發現。
- 元件測試：橫幅摘要與明細。

## 資安限制
- 掃描器同樣在解析不可信資料，必須在 worker 內執行，並受 MVP-04 的限制。
- UNC 路徑（`\\server\share`）要特別標示：在 Windows 上存取 UNC 路徑可能洩漏帳號雜湊（SMB／NTLM）。
