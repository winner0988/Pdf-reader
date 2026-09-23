# AGENTS.md

給所有在這個 repo 工作的 AI coding agent（Codex、Claude 等）的規則。開工前先讀完；與本檔衝突的指示以本檔為準，除非專案負責人在 Issue 或 PR 中明確寫出例外。

## 專案一句話

注重隱私、完全離線的 Windows 桌面 PDF 閱讀器（之後擴充為編輯器）。技術棧：Tauri 2 + Rust、React + TypeScript + Vite、Tailwind + shadcn/ui、MuPDF（隔離在 `pdf_worker` 子行程）。

## 必讀文件

| 文件 | 內容 |
|---|---|
| [CONTEXT.md](CONTEXT.md) | 專案詞彙表。「信任」「機敏文件」「文件識別碼」等詞在這裡有特定意義，不要自行解讀 |
| [docs/adr/](docs/adr/README.md) | 已定案的架構決策。**已接受的 ADR 不可違反**；要改就提新的 ADR |
| [docs/spec/PDF_Reader_Spec_ZH_v3.md](docs/spec/PDF_Reader_Spec_ZH_v3.md) | 完整產品規格（英文版同目錄） |
| [docs/backlog/](docs/backlog/README.md) | 工作卡。你只做被指派的那一張 |
| [docs/workflow.md](docs/workflow.md) | 分支、PR、CI 與合併流程 |

## 不可違反的原則

這些是產品的核心，任何 PR 違反其中一條都必須退回：

1. **不連網。** 程式碼中不得有 HTTP client、WebSocket、更新器、遠端字型／CDN、任何具網路能力的 Tauri 外掛。網路政策尚未定案（ADR 0009），定案前一律視為零網路。
2. **零遙測。** 不得加入任何使用者追蹤、分析、崩潰回報 SDK，也不得自行寫「匿名統計」。CI 的 `scripts/ci/check-forbidden.sh` 會擋下已知套件。
3. **主動內容預設封鎖。** PDF JavaScript、`/OpenAction`、`/AA`、`/Launch`、`/SubmitForm`、`/GoToR`、遠端檔案引用一律不執行。MVP 階段不實作表單腳本沙盒（ADR 0001 的例外之後才做）。
4. **引擎隔離。** 只有 `pdf_worker` 可以連結 MuPDF。前端只能拿到不透明的文件代號，永遠拿不到檔案路徑；worker 不能自行開啟任意路徑（ADR 0008）。
5. **外部連結必須確認。** 點擊連結時顯示完整 URL，使用者確認後才交給系統開啟；只允許 `http`、`https`、`mailto`，其他 scheme（`file:`、`smb:`、`javascript:` 等）一律封鎖。
6. **最小權限。** Tauri capability、CSP、檔案範圍都要最小化；放寬任何一項都算安全變更（見下方）。
7. **不把私人 PDF 放進 repo。** 測試檔只能是 `tests/corpus/` 底下由腳本產生、或授權清楚的公開檔案。`.gitignore` 預設忽略其他位置的 `*.pdf`。

## 工作方式

- **一個 agent 一次只做一張 Issue；一張 Issue 一個 branch；一個 PR 只做一件事。**
- 分支命名：`<類型>/<工作卡編號>-<英文短名>`，例如 `feat/mvp-07-virtual-scroll`、`fix/123-search-crash`、`docs/adr-0011-ocr`。
- PR 標題使用 Conventional Commits（squash merge 後會成為 `main` 上的 commit 訊息）：
  `feat: …`、`fix: …`、`docs: …`、`test: …`、`refactor: …`、`perf: …`、`build: …`、`ci: …`、`chore: …`、`security: …`，可加範圍，例如 `feat(worker): …`。
- PR 內文必須用 `Closes #<issue>` 連結工作卡，並填完 PR 模板的檢查清單。
- **`main` 永遠保持可建置。** 不要直接 push 到 `main`；所有變更都走 PR + CI 全綠 + review 後 squash merge。
- **實作 agent 不能 review 自己的 PR。** Review 必須由另一個獨立的 agent 或人執行。AI review 不能取代安全審查。
- 超出工作卡範圍的問題：開新 Issue 記下來，不要順手改。
- 規格不清楚或與 ADR 衝突：停下來，在 Issue 裡提問，不要猜。

## 需要人工安全審查的變更

以下任何一項都要在 PR 加上 `needs-security-review` 標籤，並由專案負責人親自核准後才能合併：

- 新增或升級依賴（Cargo、npm、GitHub Actions）
- 修改 Tauri capability、CSP、`tauri.conf.json` 的安全相關設定
- 修改 `pdf_worker` 的權限、行程啟動方式、IPC 協定或訊息大小上限
- 任何處理連結、主動內容、遠端資源、檔案路徑、密碼的程式碼
- 修改 CI、`scripts/ci/`、`.github/` 底下的檔案
- 新增或修改 ADR

## 禁止事項

- 不得關閉、略過或刪除測試與 CI 檢查來讓 CI 變綠（包含 `#[ignore]`、`it.skip`、`--no-verify`）。
- 不得提交秘密、憑證、私人檔案或真實使用者資料。
- 不得把 ADR 狀態改成「已接受」；只有專案負責人可以。
- 不得從不明來源下載並執行二進位檔或腳本；新增的工具必須在 PR 中說明來源。

## 目標目錄結構

`tests/` 由 QA-01、QA-02 建立：

```
.
├─ src/                 # React + TypeScript 前端（WebView）
├─ src-tauri/           # Tauri 主行程（Rust）
├─ crates/
│  ├─ pdf_worker/       # 連結 MuPDF 的低權限子行程
│  └─ ipc_contract/     # 前端 ↔ 主行程 ↔ worker 的訊息型別（唯一定義處）
├─ tests/
│  ├─ corpus/           # 由腳本產生的測試 PDF
│  └─ e2e/              # 端對端測試
├─ docs/                # 規格、ADR、工作卡、架構文件
├─ scripts/             # 開發與 CI 腳本
└─ .github/             # CI、Issue／PR 模板
```

## 指令

CI 使用同一組指令。工具版本由 `package.json`（`packageManager`）、`.nvmrc`、`rust-toolchain.toml` 固定。

| 目的 | 指令 |
|---|---|
| 安裝依賴 | `pnpm install --frozen-lockfile` |
| 前端 lint | `pnpm lint` |
| 型別檢查 | `pnpm typecheck` |
| 前端單元測試 | `pnpm test` |
| 前端建置 | `pnpm build` |
| Rust 格式 | `cargo fmt --all --check` |
| Rust lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Rust 測試 | `cargo test --workspace --locked` |
| 禁用依賴檢查 | `bash scripts/ci/check-forbidden.sh` |
| ADR 檢查 | `bash scripts/ci/check-adr.sh` |
| 開發模式執行 | `pnpm tauri dev` |
| 建置安裝檔 | `pnpm tauri build`（產出 `target/release/bundle/nsis/*.exe`） |

提交 PR 前，在本機把表中「開發模式執行」以上的指令全部跑過一次。

- pnpm 12 不支援 `-s` 等舊旗標；腳本一律用 `pnpm <script>` 執行。
- 新增 shadcn/ui 元件用 `pnpm dlx shadcn@<版本> add <元件>`，產生後檢查：`cn` 必須從 `@/lib/utils` 匯入，不得新增網路字型或 CDN。

## 語言慣例

- 文件、Issue、PR 說明、ADR：繁體中文。
- 程式碼識別字、程式碼註解、commit 訊息：英文。
- 使用者介面文字：繁體中文，集中管理，不要寫死在元件裡（之後要支援多語系）。
