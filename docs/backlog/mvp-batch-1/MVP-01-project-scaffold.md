---
title: "[MVP-01] 專案骨架：Tauri 2 + React + Rust workspace，啟用 CI"
labels: task,mvp,area:build,agent:release,needs-security-review
---

- **需求 ID**：MVP-R11（基礎建設）；ADR 0007
- **負責角色**：Review／Release agent
- **相依**：無

## 目標
建立可建置、可在 Windows 開出空白視窗的 Tauri 2 專案，並讓 CI 的 `Rust (Windows)` 與 `Frontend` job 實際生效。

## 範圍
- 依 AGENTS.md「目標目錄結構」建立：
  - 根目錄 `package.json`（pnpm，必須有 `packageManager` 欄位）、`pnpm-lock.yaml`、`.nvmrc`（Node LTS）
  - Vite + React + TypeScript（`strict: true`）、Tailwind CSS、shadcn/ui 初始化
  - `src-tauri/`（Tauri 2）
  - 根目錄 `Cargo.toml` workspace，成員：`src-tauri`、`crates/pdf_worker`（先只是一個會印出版本的 binary）、`crates/ipc_contract`（空 library）
  - `rust-toolchain.toml`（stable，含 rustfmt、clippy）
- `package.json` scripts：`dev`、`build`、`lint`（ESLint）、`typecheck`（`tsc --noEmit`）、`test`（Vitest）、`tauri`。
- 各一個最小的 Rust 單元測試與 Vitest 測試，證明測試管線有效。
- 調整 `.github/workflows/ci.yml`，讓 Tauri crate 在 CI 可以編譯（若需要前端建置產物，先在 Rust job 建置前端）。
- `.github/dependabot.yml` 加入 `cargo` 與 `npm`（commit 前綴分別用 `build(deps)`）。
- 把 AGENTS.md「指令」表更新成實際可用的指令；README 加上本機開發步驟。

## 不做什麼
- 任何 PDF 功能、MuPDF、IPC 合約。
- UI 設計（畫面只需顯示應用程式名稱）。
- 安裝包簽章、自動更新、檔案關聯。

## 可動的模組
- 根目錄設定檔（`package.json`、`Cargo.toml`、`rust-toolchain.toml`、`.nvmrc`、`tsconfig*.json`、ESLint／Vite／Tailwind 設定等）
- `src/`、`src-tauri/`、`crates/`
- `.github/workflows/ci.yml`、`.github/dependabot.yml`
- `AGENTS.md`（僅「指令」表）、`README.md`（僅開發步驟）、`.gitignore`

## 驗收情境
- 假設是乾淨的 Windows 11 開發環境，當依 README 執行 `pnpm install` 與 `pnpm tauri dev`，則會開出顯示應用程式名稱的視窗。
- `pnpm tauri build` 能產出（未簽章的）安裝檔。
- 此 PR 上 `Rust (Windows)` 與 `Frontend` job 實際執行且通過（不是被略過）。
- `Guardrails` 仍通過。

## 必跑測試
- AGENTS.md「指令」表中的全部指令。

## 資安限制
- `tauri.conf.json` 設定嚴格 CSP：`default-src 'self'`，不得出現任何外部來源；`connect-src` 只允許 Tauri IPC 必要的來源。
- `src-tauri/capabilities/` 只保留最少權限；不得啟用 fs、shell、http 類外掛的前端權限。
- 不得引入網路字型；若 shadcn/ui 或範本預設引用 Google Fonts，改用系統字型堆疊。
- PR 中列出所有新增的依賴與用途（本卡會新增大量依賴，因此需要安全審查）。
