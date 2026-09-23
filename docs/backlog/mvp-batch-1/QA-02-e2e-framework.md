---
title: "[QA-02] E2E 測試框架（Windows）"
labels: task,mvp,qa,area:ci,agent:qa,needs-security-review
---

- **需求 ID**：MVP 整體驗收（MVP-R1、R11 起步）
- **負責角色**：QA／安全 agent
- **相依**：MVP-06

## 目標
在 Windows CI 上以真實應用程式執行端對端測試，之後每張功能卡都能加上 E2E 驗收。

## 範圍
- 選定方案並寫短評估（`docs/architecture/e2e.md`）：
  - Tauri 官方 WebDriver（`tauri-driver` + Edge WebDriver）操作真實應用程式，或
  - Playwright 針對前端＋模擬 IPC 的 UI 層測試
  - 可以兩者並用，但要說明各自負責什麼
- 第一批 E2E：
  - 啟動 → 空狀態
  - 以命令列參數開啟 QA-01 的一般 PDF → 顯示正確頁數
  - 開啟損毀檔 → 錯誤狀態
- 新增 CI job `E2E (Windows)`，更新 `docs/workflow.md` 的檢查表。
- 失敗時把截圖與日誌上傳為 artifact。

## 不做什麼
- 視覺回歸截圖比對、效能測試。

## 可動的模組
- `tests/e2e/`
- `.github/workflows/ci.yml`
- `package.json`（E2E script）
- `docs/architecture/e2e.md`、`docs/workflow.md`

## 驗收情境
- CI 上 `E2E (Windows)` 穩定通過（同一 commit 連跑 3 次沒有不穩定失敗）。
- 失敗時可以從 artifact 取得截圖與日誌。

## 必跑測試
- 新的 E2E 測試本身。

## 資安限制
- WebDriver 等工具只能從官方來源取得，版本固定並在 PR 說明。
- artifact 只能包含 `tests/corpus/` 的檔案產生的畫面，保留天數設短（例如 7 天）。
- 本卡修改 CI，需要安全審查。
