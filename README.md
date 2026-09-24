# Pdf-reader

注重隱私、完全離線的桌面 PDF 閱讀器（之後擴充為編輯器）。不連網、無遙測、PDF 主動內容預設封鎖，PDF 引擎隔離在低權限子行程中執行。

> **狀態：開發初期。** 目前只有專案骨架（空白視窗），還不能開啟 PDF；進度見 [工作卡](docs/backlog/README.md)。

## 本機開發（Windows）

需要先安裝：

- [Node.js](https://nodejs.org/) 22 LTS（版本見 `.nvmrc`）
- [pnpm](https://pnpm.io/) 12：`npm install -g pnpm@12`
- [Rust](https://rustup.rs/)（rustup；實際版本由 `rust-toolchain.toml` 自動安裝）
- Visual Studio Build Tools，勾選「使用 C++ 的桌面開發」
- [LLVM](https://llvm.org/)：`winget install LLVM.LLVM`（建置 MuPDF 需要，安裝在預設位置即可；見 [docs/architecture/mupdf-binding.md](docs/architecture/mupdf-binding.md)）
- Microsoft Edge WebView2（Windows 11 已內建）

```bash
pnpm install
cargo build -p pdf_worker   # 第一次，或 worker 有變更時；開發模式的主程式在 target/debug/ 旁邊找 pdf_worker.exe
pnpm tauri dev
```

`pnpm bundle` 會在 `target/release/bundle/nsis/` 產出包含 `pdf_worker.exe` 的安裝檔（尚未簽章），見 [docs/architecture/packaging.md](docs/architecture/packaging.md)。其他檢查指令見 [AGENTS.md](AGENTS.md#指令)。

## 技術棧

Windows-first · Tauri 2 + Rust · React + TypeScript + Vite · Tailwind + shadcn/ui · MuPDF（`pdf_worker` 子行程）· SQLite · Windows Credential Manager。理由見 [ADR 0007](docs/adr/0007-tech-stack-and-platform.md)。

## 文件導覽

| 文件 | 用途 |
|---|---|
| [規格書（繁中）](docs/spec/PDF_Reader_Spec_ZH_v3.md)／[Spec (EN)](docs/spec/PDF_Reader_Spec_EN_v3.md) | 完整產品需求 |
| [CONTEXT.md](CONTEXT.md) | 專案詞彙表 |
| [docs/adr/](docs/adr/README.md) | 架構決策紀錄 |
| [docs/backlog/](docs/backlog/README.md) | MVP 第一批工作卡 |
| [docs/workflow.md](docs/workflow.md) | 開發流程、分支與 CI 規則、GitHub 設定步驟 |
| [AGENTS.md](AGENTS.md) | AI coding agent 的工作規則 |
| [docs/planning/](docs/planning/2026-09-23-kickoff-plan.md) | 開發啟動計畫 |

## MVP 範圍

1. 開啟本機 PDF
2. MuPDF 在隔離子行程中渲染頁面
3. 虛擬滾動、縮放、旋轉、目錄、搜尋
4. 預設封鎖 PDF JavaScript、遠端資源與外部連結；連結要顯示完整 URL 並經確認
5. 基本 Tauri UI、離線運作、無遙測

OCR、Word 轉檔、完整編輯、簽章、加密、批次背景服務、表單腳本沙盒都**不在** MVP 內；每一項都要先做 POC 並寫 ADR。

## 開發流程

```mermaid
flowchart LR
  A[負責人：MVP／UX／驗收決策] --> B[AI Lead：拆 Issue 與架構]
  B --> C[實作 Agent：單一功能分支]
  C --> D[CI：建置、測試、安全掃描]
  D --> E[獨立 AI Review]
  E --> F[QA／安全驗收]
  F --> G[Squash merge 到 main]
```

細節見 [docs/workflow.md](docs/workflow.md)。

## 安全

請勿在公開管道張貼可利用的惡意 PDF；回報方式見 [SECURITY.md](SECURITY.md)。

## 授權

本專案以 [GNU Affero General Public License v3.0 或之後的版本](LICENSE)（AGPL-3.0-or-later）授權，理由與散布時的義務見 [ADR 0011](docs/adr/0011-license-and-distribution.md)。

PDF 引擎 [MuPDF](https://mupdf.com/) 由 Artifex Software 以 AGPL-3.0 授權（另有商業授權）；其他第三方套件保留各自的授權。
