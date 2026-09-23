---
title: "[MVP-03] MuPDF 建置與渲染 POC（Windows + CI）"
labels: task,mvp,area:worker,agent:core,needs-security-review
---

- **需求 ID**：MVP-R2；ADR 0003
- **負責角色**：核心 agent
- **相依**：MVP-01

## 目標
證明 MuPDF 可以在 Windows 本機與 CI 上以 Rust 建置並渲染頁面，並確定綁定方式。

## 範圍
- 評估並選定綁定方式：現有 Rust crate，或以 bindgen 自行包裝 C API。評估項目：Windows 建置、維護狀態、授權、建置時間、能否在編譯期移除 JavaScript 引擎。
- 在 `crates/pdf_worker` 內加入最小功能：開啟 PDF、取得頁數、把指定頁渲染成點陣圖。
- MuPDF 固定版本，從官方來源取得，並以 commit SHA 或 checksum 驗證；不得使用來路不明的預編譯檔或鏡像。
- 編譯期關閉 MVP 不需要的功能，**特別是 JavaScript 引擎**（例如 MuPDF 的 `FZ_ENABLE_JS=0`，若綁定允許）。
- CI 快取 MuPDF 建置結果，控制 Windows job 時間。
- 評估結果寫在 `docs/architecture/mupdf-binding.md`：MuPDF 版本、取得方式、綁定選擇、建置時間、授權義務（連到 DEC-02）。

## 不做什麼
- 子行程隔離、IPC、UI、搜尋、目錄。

## 可動的模組
- `crates/pdf_worker/`
- `docs/architecture/mupdf-binding.md`
- `.github/workflows/ci.yml`（快取與建置依賴）
- 根目錄 `Cargo.toml`（workspace 依賴）

## 驗收情境
- `cargo test -p pdf_worker` 在 Windows CI 上開啟一份簡單 PDF（QA-01 的檔案，或測試中即時產生），頁數正確、輸出影像尺寸正確、內容不是全白。
- 文件記錄乾淨建置與快取後建置的時間。
- 程式中沒有任何啟用 PDF JavaScript 的呼叫。

## 必跑測試
- `cargo test -p pdf_worker`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`

## 資安限制
- 固定版本並驗證來源；升級 MuPDF 一律需要安全審查。
- 若無法在編譯期移除 JavaScript 引擎，文件中說明原因，並確保程式碼從不呼叫啟用 JS 的 API（MVP-11 會加回歸測試）。
