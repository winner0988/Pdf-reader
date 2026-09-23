---
title: "[MVP-12] 外部連結攔截：顯示完整 URL 並確認"
labels: task,mvp,area:ui,area:security,agent:frontend,agent:core,needs-security-review
---

- **需求 ID**：MVP-R9
- **負責角色**：前端 agent（疊加層與對話框）＋核心 agent（主行程開啟連結）
- **相依**：MVP-07

## 目標
點擊文件中的連結時，內部連結直接跳頁；外部連結一定先讓使用者看到完整 URL 並確認，危險的連結一律封鎖。

## 範圍
- worker 回傳每頁的連結：區域與目標類型（內部頁、URI、其他動作）。
- 頁面上的透明連結疊加層；滑鼠移上時在狀態列顯示目標。
- 內部連結：直接跳頁。
- URI：確認對話框
  - 完整顯示 URL（不截斷、可捲動），醒目標示 scheme 與主機名稱
  - 國際化網域同時顯示 punycode 並加上警示
  - 按「開啟」才由主行程交給系統預設瀏覽器；另有「複製連結」
- scheme 白名單：`http`、`https`、`mailto`。其他（`file:`、`smb:`、`javascript:`、`data:`、`ms-*` 等 Windows 協定處理常式）封鎖並說明原因。
- `/Launch`、`/GoToR`、UNC 目標：一律封鎖並說明原因。
- 「開啟連結」的 IPC 指令只接受 worker 回報過的連結 ID，不接受前端傳來的任意字串；主行程再驗證一次 scheme。

## 不做什麼
- 「永遠信任此網域」、連結預覽、編輯連結。

## 可動的模組
- `crates/pdf_worker/`（連結擷取）
- `crates/ipc_contract/`
- `src-tauri/src/`（開啟連結）
- `src/features/links/`、`src/features/viewer/`

## 驗收情境
- 點 https 連結 → 對話框顯示完整 URL → 取消則什麼都不發生；確認則以系統瀏覽器開啟。
- `javascript:`、`file:///C:/Windows/...`、`\\share.example.invalid\x`、`ms-msdt:`、`search-ms:` 連結 → 被封鎖，不啟動任何程式。
- 10,000 字元的 URL、含 RTL 覆寫字元的 URL、同形異義網域（例如用西里爾字母 а 的 `аpple.com`）→ 正確顯示並警示。
- 測試證明前端無法透過 IPC 要求開啟任意 URL。

## 必跑測試
- 主行程單元測試：scheme 白名單、連結 ID 驗證、IDN 與控制字元處理。
- 元件測試：確認對話框的各種 URL 顯示。

## 資安限制
- 在 Rust 端開啟連結；不得把 shell／opener 權限開放給前端。
- URL 以單一參數交給系統開啟，不得組成命令列字串（避免命令注入）。
