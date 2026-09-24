# 打包與安裝

對應工作卡 REL-01、ADR 0008。說明安裝檔怎麼帶上 `pdf_worker.exe`、worker 為什麼不需要 VC++ 執行階段，以及 CI 怎麼驗證。

## 指令

```bash
pnpm bundle    # 產出 target/release/bundle/nsis/PDF Reader_<版本>_x64-setup.exe
```

`pnpm bundle` 等於 `tauri build --config src-tauri/tauri.bundle.conf.json`。直接執行 `pnpm tauri build` 產出的安裝檔**不含 worker**，不要拿來發佈。

## 流程

1. `beforeBuildCommand`（`src-tauri/tauri.bundle.conf.json`）先建置前端，再執行 `scripts/release/build-worker.mjs`：
   - 以 release profile、`--target x86_64-pc-windows-msvc` 建置 `pdf_worker`，C／C++ 執行階段全部靜態連結（見下節）；
   - 檢查匯入表，只要還依賴 VC++ 可轉散發套件的 DLL 就失敗；
   - 複製到 `src-tauri/binaries/pdf_worker-x86_64-pc-windows-msvc.exe`（已列入 `.gitignore`）。
2. Tauri 以 `bundle.externalBin` 把它打包，安裝時放在主程式旁邊，檔名 `pdf_worker.exe`。
3. 主行程以 `worker_host::bundled_worker_path()`（目前執行檔所在目錄下的 `pdf_worker.exe`）找到 worker。開發模式下主程式在 `target/debug/`，`cargo build -p pdf_worker`（或 `cargo build --workspace`）會把 worker 建置到同一個目錄。

為什麼 `externalBin` 放在獨立的設定檔：`tauri-build` 在**每次**編譯主程式時都會檢查 `externalBin` 指到的檔案存在。如果寫在 `tauri.conf.json`，`cargo clippy --workspace`、`cargo test --workspace` 都得先建置一次 release worker。

## 不依賴 VC++ 執行階段

乾淨的 Windows 11 不保證裝有 VC++ 可轉散發套件。

| 程式 | 做法 | 匯入的執行階段 |
|---|---|---|
| `pdf_worker.exe` | Rust 以 `+crt-static` 建置；MuPDF 的 MSBuild 專案寫死 `/MD`，由 `scripts/release/static-crt.props` 透過 MSBuild 的 `ForceImportBeforeCppTargets` 改成 `/MT`；mupdf-sys 的 C wrapper 由 `cc` 依 `crt-static` 自動使用 `/MT` | 無（C、C++ 執行階段都靜態連結；MuPDF 內的 harfbuzz 是 C++） |
| 主程式 | `tauri build` 設定 `STATIC_VCRUNTIME=true`，`tauri-build` 靜態連結 vcruntime | 只有 Windows 內建的 UCRT（`ucrtbase.dll`／`api-ms-win-crt-*`） |

`scripts/release/check-imports.mjs <exe>` 會列出匯入的 DLL，並在出現 `MSVCP*`、`VCRUNTIME*`、`ucrtbased` 等 DLL 時失敗。

注意：`_CL_=/MT` 行不通，因為 clang-cl 也會讀 `_CL_`，把 `/MT` 當成檔名。mupdf-sys 的 wrapper 在本機是用 clang-cl 編譯的（見 [mupdf-binding.md](mupdf-binding.md)）。

## 安裝位置與權限

- `installMode: perMachine`：安裝到 `C:\Program Files\PDF Reader\`，只有系統管理員可寫入，所以一般使用者權限的程式無法替換 `pdf_worker.exe`。代價是安裝時需要 UAC 提權。
- Program Files 允許所有 app package 讀取與執行，所以 AppContainer 可以直接啟動 worker，不需要修改檔案 ACL（見 [worker-sandbox.md](worker-sandbox.md#appcontainer)）。
- 解除安裝時，`src-tauri/windows/installer-hooks.nsh` 呼叫 `DeleteAppContainerProfile("PdfReader.Worker")`，刪除 worker 的 AppContainer profile。它只能刪除執行解除安裝程式的那位使用者的 profile；同一台電腦上其他使用者的 profile（空資料夾與登錄機碼對應）會留下。`crates/sandbox/tests/installer_hooks.rs` 確認腳本中的名稱與 `sandbox::WORKER_APP_CONTAINER` 一致。

## WebView2

安裝檔**永遠不下載** WebView2 Runtime（REL-02，[#37](https://github.com/winner0988/Pdf-reader/issues/37)，負責人 2026-09-24 決定）。

- **支援範圍**：只支援 Windows 11，它內建 WebView2。
- **設定**：`tauri.conf.json` 的 `bundle.windows.webviewInstallMode` 是 `skip`。
  - Tauri 的預設 `downloadBootstrapper` 會在電腦缺少 WebView2 時，於安裝過程向 Microsoft 下載，違反「不連網」原則。
  - `embedBootstrapper` 也會在安裝時下載。
- **缺少 WebView2 時**（被移除，或在 Windows 10 上安裝）：
  - 安裝檔：`installer-hooks.nsh` 的 `NSIS_HOOK_PREINSTALL` 以與 Tauri 相同的登錄檔檢查偵測，顯示說明後繼續安裝；靜默安裝（`/S`）不會停下來。
  - app：啟動時以 `tauri::webview_version()` 檢查，失敗就以原生對話框說明缺少什麼、到哪裡下載，然後結束，不會無聲無息地關閉。文字在 `src-tauri/src/strings.rs`。
  - 兩處都只顯示 Microsoft 官方網址，不代為下載。
- **守門**：
  - `check-security-config.mjs`（`Guardrails`）只接受不連網的模式：`skip`、`offlineInstaller`、`fixedRuntime`；
  - `installer.yml` 確認產生的安裝腳本不是會下載的模式（`downloadBootstrapper`、`embedBootstrapper`）。Tauri 的範本把每種模式的程式碼都留在腳本中，以編譯期的 `!if` 排除，所以要檢查模式，而不是搜尋下載網址。`skip` 時 Tauri 讓 `INSTALLWEBVIEW2MODE` 留空。
- `installer-hooks.nsh` 必須以 UTF-8（含 BOM）儲存，否則 NSIS 會以系統字碼頁讀取，中文會變成亂碼。

## PDF 關聯與預設程式

REL-03（[#68](https://github.com/winner0988/Pdf-reader/issues/68)），規格 §1「作業系統關聯」。

- **安裝後**（`installer-hooks.nsh` 的 `NSIS_HOOK_POSTINSTALL`，64 位元登錄檔視圖）：
  - ProgID `PdfReader.Document`：圖示與開啟指令 `"…\pdf-reader.exe" "%1"`；
  - `.pdf` 的 `OpenWithProgids` 與 `Applications\pdf-reader.exe`：出現在檔案總管的「開啟檔案」；
  - `Software\PDF Reader\Capabilities` 與 `RegisteredApplications`：出現在 Windows「預設應用程式」。
- **不搶預設**：
  - 安裝檔不改 `.pdf` 本身的預設值；
  - Windows 10／11 只能由使用者在設定中指定預設程式。
  - 所以沒有使用 Tauri 的 `bundle.fileAssociations`：它會直接改寫 `.pdf` 的預設值。
- **設為預設**：
  - app 的「⋯」選單中的「設為預設 PDF 閱讀器」，由主行程以固定網址開啟 `ms-settings:defaultapps?registeredAppMachine=PDF%20Reader`，也就是 Windows 設定中 PDF Reader 的頁面；
  - 沒有這個頁面的 Windows 版本會顯示預設應用程式清單；
  - 開不了時，對話框說明手動的路徑。
- **視窗標題**：
  - 顯示開啟中的檔名（`檔名 - PDF Reader`），由主行程在開啟與關閉文件後設定；
  - 檔名不含資料夾。
- **解除安裝**（`NSIS_HOOK_POSTUNINSTALL`）：移除以上所有項目。
  - 使用者若把 PDF Reader 設為預設，Windows 會在 ProgID 消失後自行改回；
  - 更新時先解除安裝再安裝，ProgID 名稱不變，使用者的選擇會保留。

## CI

`.github/workflows/installer.yml`（push 到 `main`、手動觸發，以及變更會進入安裝檔的 PR）：

1. `pnpm bundle` 建置安裝檔。
2. 確認安裝腳本不會下載 WebView2（見上方「WebView2」）。
3. 以 7-Zip 列出安裝檔內容，確認包含 `pdf_worker.exe`。
4. `/S` 靜默安裝（per machine），並確認 PDF 關聯已註冊、`.pdf` 的預設值沒有被改成 PDF Reader。
5. 對安裝後的所有 `.exe` 執行 `check-imports.mjs`。
6. `worker_smoke`（`crates/worker_host/src/bin/worker_smoke.rs`）以沙盒啟動**安裝後的** worker，完成握手、開啟並渲染一頁 PDF。
7. 靜默解除安裝，確認 AppContainer profile 的資料夾與 PDF 關聯都已移除。

CI runner 裝有 VC++ 執行階段，所以「在乾淨的 Windows 11 上能執行」是靠第 5 步的匯入表檢查來保證，不是實際在乾淨環境上執行。

## 剩餘風險

| 風險 | 後續 |
|---|---|
| 安裝檔尚未簽章，SmartScreen 會警告 | 程式碼簽章另開卡 |
| 沒有在真正乾淨的 Windows 11 VM 上實測安裝 | 發佈前的手動驗收清單 |
| 其他使用者的 AppContainer profile 在解除安裝後留下 | 內容為空，影響很小；如需處理，可在主程式啟動時清理 |
| 缺少 WebView2 的電腦（被移除，或 Windows 10）無法使用 | 安裝檔與 app 都會說明，由使用者自行安裝；CI runner 裝有 WebView2，所以這兩段說明沒有在 CI 上實際顯示過 |
