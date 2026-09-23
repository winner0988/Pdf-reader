---
title: "[MVP-02] IPC 合約 v0 與行程模型文件"
labels: task,mvp,area:ipc,agent:core,needs-security-review
---

- **需求 ID**：MVP-R2；ADR 0008
- **負責角色**：核心 agent
- **相依**：MVP-01

## 目標
定義前端 ↔ 主行程 ↔ `pdf_worker` 之間的訊息與資料格式，讓後續卡片可以平行開發而不互相猜測。

## 範圍
- `docs/architecture/ipc-contract.md`：
  - 行程模型與信任邊界圖（WebView／主行程／worker）
  - 每個訊息的方向、欄位、錯誤、大小上限、是否可取消
  - 版本握手與不相容時的行為
  - 頁面影像的傳輸方式（例如 Tauri 原生二進位回應 vs. 自訂 URI scheme），選一種並說明理由與量測計畫
- 最小訊息集：
  - 開啟文件（主行程交付已開啟的檔案 → 回傳 `DocumentInfo`：頁數、每頁尺寸、是否有目錄、安全發現摘要）
  - 渲染頁面（文件、頁碼、縮放、旋轉 → 影像）
  - 取得目錄、取得頁面連結
  - 搜尋（串流回傳命中頁碼與區域）
  - 取消請求、關閉文件
- `crates/ipc_contract`：Rust 型別（serde）；前端 TypeScript 型別由 Rust 產生（ts-rs 或 specta 擇一並說明）。
- worker 協定：長度前綴 framing、單一訊息大小上限。
- CI 檢查產生的 TS 型別與提交內容一致。

## 不做什麼
- 實作 worker、渲染、搜尋或任何 UI。

## 可動的模組
- `docs/architecture/`
- `crates/ipc_contract/`
- `src/ipc/`（僅限產生的型別與薄包裝）
- `package.json`（產生型別的 script）、`.github/workflows/ci.yml`（一致性檢查）

## 驗收情境
- 文件列出每個訊息的方向、欄位、錯誤與上限。
- 所有訊息型別的序列化／反序列化 round-trip 測試通過；超過大小上限或版本不符的訊息被拒絕。
- 前端 TS 型別由 Rust 產生，修改 Rust 型別但沒重新產生時 CI 失敗。
- 前端可見的型別中**沒有任何檔案路徑欄位**。

## 必跑測試
- `cargo test -p ipc_contract`（round-trip、超大訊息、版本不符）
- 型別產生一致性檢查

## 資安限制
- 協定中不得存在「讀取任意檔案」、「執行指令」、「開啟任意 URL」這類訊息。
- 所有來自 worker 的數值（頁數、尺寸、座標、字串長度、陣列長度）都要有上限並驗證。
- 反序列化遇到未知版本或未知欄位時拒絕，而不是忽略。
