# ADR 0007：技術棧與首發平台

## 狀態
已接受（2026-09-23，依 [開發啟動計畫](../planning/2026-09-23-kickoff-plan.md) 定案）

## 背景
規格書 §1 仍把 Tauri、Electron、Qt 並列為候選框架，前端技術、本機資料儲存與開發流程也尚未定案。沒有定案的技術棧，AI agent 會各自選擇工具，造成風格與依賴失控。

## 決定
| 層 | 選擇 |
|---|---|
| 首發平台 | Windows-first；macOS／Linux 之後再做 |
| 桌面框架 | Tauri 2 + Rust（Electron、Qt 不再考慮） |
| 前端 | React + TypeScript + Vite；Tailwind CSS + shadcn/ui |
| PDF 核心 | MuPDF（ADR 0003），包在獨立低權限行程 `pdf_worker`（ADR 0008） |
| 本機資料 | SQLite（設定、信任、佇列、索引）＋ Windows Credential Manager（密碼，ADR 0006） |
| 版控與工作管理 | GitHub 私有 repo + Issues + Projects + Pull Requests |
| CI | GitHub Actions：格式化、lint、型別、單元測試、E2E、依賴／秘密掃描 |
| AI 開發 | 實作 agent 與 review agent 必須是不同的 agent，不能審自己的 PR |

不需要雲端後端；Tauri 是桌面殼與 Rust 原生層，前端完全在本機 WebView 中執行。

## 後果
- 換來：全專案只有一套技術棧，agent 可以依 `AGENTS.md` 直接開工；Rust 讓原生層具備記憶體安全。
- 付出：Windows-first 代表 macOS Keychain、Linux Secret Service 等跨平台工作延後，屆時需要補測。
- 付出：前端跑在 WebView，必須靠 Tauri capability 與 CSP 縮小權限；這不能保護 Rust 程式碼或 PDF 引擎本身的漏洞（見 ADR 0008）。
