---
title: "[MVP-04] pdf_worker 隔離子行程：低權限啟動、崩潰隔離、重啟"
labels: task,mvp,area:worker,area:security,agent:core,needs-security-review
---

- **需求 ID**：MVP-R2；ADR 0008
- **負責角色**：核心 agent（QA／安全 agent 參與 review）
- **相依**：MVP-02、MVP-03

## 目標
主行程以最低權限啟動 `pdf_worker`，依 IPC 合約溝通；worker 崩潰、逾時或超出記憶體上限時，主行程不受影響並能恢復。

## 範圍
- 主行程的 worker 管理器：啟動、版本握手、請求逾時、取消、崩潰偵測與自動重啟、應用程式關閉時清理。
- Windows 隔離：
  - Job Object：關閉時一併結束、記憶體上限、禁止產生子行程
  - 評估 restricted token／low integrity level／AppContainer，選定採用的層級，寫進 `docs/architecture/worker-sandbox.md`（採用什麼、為什麼、還剩哪些風險）
- 檔案交付：主行程以唯讀方式開啟檔案後交給 worker（繼承 handle 或串流）；worker 不接受路徑字串。
- worker 啟動時不繼承不必要的 handle 與環境變數。
- 依 MVP-02 的上限驗證 worker 回傳的所有資料。

## 不做什麼
- UI、渲染效能最佳化、搜尋、目錄。

## 可動的模組
- `src-tauri/src/worker/`（新增）
- `crates/pdf_worker/`
- `crates/ipc_contract/`（只能補充，不能改變 MVP-02 已定義的語意）
- `docs/architecture/worker-sandbox.md`

## 驗收情境
- 主行程可以要求 worker 開啟文件並取得頁數。
- 在測試中強制結束 worker → 主行程回報可辨識的錯誤、不崩潰；下一個請求自動重啟 worker。
- worker 超過記憶體上限被終止 → 同上。
- worker 無法產生子行程（以測試驗證 Job Object 限制）。
- 協定中沒有能讓 worker 開啟任意路徑的訊息。
- `worker-sandbox.md` 列出 worker 最終的權限（integrity level、Job 限制、網路狀態）。

## 必跑測試
- 整合測試：崩潰、重啟、逾時、超大訊息、記憶體上限（Windows 專屬測試以 `cfg(windows)` 標示，在 CI 執行）。

## 資安限制
- 這是核心安全卡。任何權限放寬都要在 PR 中說明理由。
- Windows 無法單靠 Job Object 阻斷網路：文件中記錄 worker 如何做到無網路（例如 AppContainer 不給網路能力；至少要確保 worker 不連結任何網路程式庫，並由 CI 依賴檢查把關）。
