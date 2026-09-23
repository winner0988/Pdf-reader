# pdf_worker 沙盒

對應工作卡 MVP-04、SEC-01、ADR 0008。說明 `pdf_worker` 以什麼權限執行、主行程如何與它溝通，以及還剩哪些風險。

## 元件

| crate | 角色 | 是否有 `unsafe` |
|---|---|---|
| `crates/sandbox` | 以受限權限啟動子行程；本專案所有行程隔離用的 Win32 FFI 集中在這裡 | 有（每個區塊附 SAFETY 說明） |
| `crates/worker_host` | 主行程端：啟動、握手、送出請求、驗證回應、逾時、崩潰後重啟、交付檔案 | 無 |
| `crates/pdf_worker` | worker 本身：`serve` 迴圈 + MuPDF 引擎 | 只有 `handle.rs`：把收到的 handle 值轉成 `File` |

`worker_host` 會在 MVP-06 接到 Tauri 命令上。

## worker 的權限（最終狀態）

| 層 | 設定 | 效果 | 驗證（測試） |
|---|---|---|---|
| Job Object | `KILL_ON_JOB_CLOSE` | 主行程關閉或崩潰時 worker 一併結束 | `dropping_the_handle_kills_the_process` |
| | `ACTIVE_PROCESS = 1` ＋ 子行程政策 `CHILD_PROCESS_RESTRICTED` | 無法產生子行程 | `cannot_start_child_processes` |
| | `PROCESS_MEMORY`（預設 2 GiB） | 超過就配置失敗；worker 崩潰後由主行程重啟 | `memory_limit_is_enforced`、`memory_limit_contains_a_huge_render` |
| | `DIE_ON_UNHANDLED_EXCEPTION` | 崩潰不跳 Windows 錯誤回報視窗 | — |
| | UI 限制：桌面、剪貼簿讀寫、全域 atom、系統參數、他人視窗 handle、登出 | 碰不到使用者的桌面與剪貼簿 | 由 win32k 停用涵蓋 |
| Token | `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)` | 除 SeChangeNotify 外所有特權移除 | — |
| | 完整性等級 **Low**（S-1-16-4096） | 無法寫入使用者擁有的任何檔案、登錄機碼 | `runs_at_low_integrity`、`worker_runs_at_low_integrity_in_an_app_container`、`cannot_write_to_user_locations` |
| AppContainer | profile `PdfReader.Worker`，**不授予任何 capability** | 作業系統拒絕所有網路連線（包含 127.0.0.1）；讀不到沒有明確授權給 app package 的檔案，也就是使用者的所有檔案 | `network_is_blocked_even_to_localhost`、`cannot_read_user_files`、`worker_runs_at_low_integrity_in_an_app_container` |
| 行程緩解措施 | 停用 win32k 系統呼叫 | 無視窗、GDI、剪貼簿、輸入；`user32.dll` 無法載入 | `win32k_is_unavailable` |
| | 禁止動態程式碼（ACG） | 無法產生可寫又可執行的記憶體 | — |
| | 強制 ASLR、bottom-up／high-entropy ASLR、堆積損毀即終止 | 增加利用難度 | — |
| | 嚴格 handle 檢查 | 使用無效 handle 會讓 worker 立即結束 | — |
| | 禁止從遠端共用與 Low 標籤位置載入映像檔、優先 System32 | 防 DLL 植入 | — |
| | 停用舊式擴充點 | 防 AppInit DLL 等注入 | — |
| 繼承 | `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` | 只繼承 stdin／stdout／stderr 三個 pipe | — |
| | 最小環境變數（`SystemRoot`、`LOCALAPPDATA`） | 不帶使用者的其他設定；`LOCALAPPDATA` 是 AppContainer 必要的（見下方），Windows 會把它與新增的 `TEMP`、`TMP` 改指向容器自己的資料夾 | `environment_is_minimal` |
| | `DETACHED_PROCESS` | 沒有 console（Low 完整性無法建立 console，會以 `STATUS_DLL_INIT_FAILED` 失敗） | 所有 sandbox 測試 |
| 連結 | worker 執行檔不匯入任何網路或 GUI DLL | 見下方「網路」 | `worker_binary_imports_no_networking` |

評估過的層級與選擇：

| 選項 | 能擋住 | 結論 |
|---|---|---|
| Job Object | 子行程、資源濫用、UI | ✅ 採用 |
| Restricted token + Low 完整性 | 寫入使用者資料、特權操作 | ✅ 採用 |
| 行程緩解措施 | 常見的漏洞利用手法、DLL 注入 | ✅ 採用 |
| AppContainer | 以上全部，加上**網路**與讀取使用者檔案 | ✅ 採用（SEC-01） |
| Less Privileged AppContainer（LPAC） | 另外擋住只授權給「所有應用程式套件」的系統位置 | ⏭ 之後評估（見「剩餘風險」） |

## 檔案交付

1. 主行程以唯讀方式開啟使用者選擇的檔案（檢查大小上限 512 MiB）。
2. `DuplicateHandle` 把 handle 複製進 worker，存取權限只有 `FILE_GENERIC_READ`（即使主行程的 handle 可寫）。
3. `Open` 請求只帶 handle 值（`FileHandle(u64)`），**worker 永遠拿不到路徑**。
4. worker 讀入記憶體（同樣有 512 MiB 上限）後交給 MuPDF。

驗證：`duplicated_file_handles_are_read_only`（讀得到、寫不進去）、`opens_and_renders_through_the_sandbox`。

AppContainer 不影響這個流程：handle 在複製時就已帶著存取權限，worker 使用時不會再做路徑層級的存取檢查。

## AppContainer

- `Sandboxed::spawn` 第一次執行時以 `CreateAppContainerProfile` 建立 profile（不要求任何 capability），之後改用 `DeriveAppContainerSidFromAppContainerName` 取得同一個 SID。建立 profile 與修改 ACL 都以行程內的鎖序列化，避免並行建立失敗。
- profile 會寫入使用者設定檔：`HKCU\Software\Classes\Local Settings\...\AppContainer\Mappings\<SID>` 與 `%LOCALAPPDATA%\Packages\pdfreader.worker\`。這是容器自己的資料夾（`TEMP`、`TMP` 指向這裡），不含使用者資料。
- 解除安裝時要呼叫 `sandbox::delete_app_container_profile`，清掉上述兩處（`app_container_profile_can_be_deleted`）。接到安裝檔是 REL-01（#34）的工作。
- AppContainer 只能開啟有明確授權給它的檔案。worker 執行檔若放在使用者擁有的位置（開發時的 `target\`、每位使用者各自安裝的目錄），啟動前會把**該執行檔本身**（不含所在資料夾）的讀取＋執行權限授予容器 SID；若沒有權限修改 ACL（例如裝在 Program Files，那裡本來就允許所有 app package 讀取），就略過。
- 工作目錄設為 `System32`：容器讀不到執行檔所在的資料夾，worker 也不使用相對路徑。
- `CreateProcess` 在 AppContainer 模式下需要 `LOCALAPPDATA`，否則以 `ERROR_ENVVAR_NOT_FOUND` 失敗；Windows 會把它改寫成容器的資料夾，所以不會把使用者的真實路徑交給 worker。
- 系統字型（`%SystemRoot%\Fonts`）授權給所有 app package，容器可以讀取（`system_fonts_are_readable`），不影響 #31 的字型處理。

## 通訊與失效處理

- 啟動後第一個 frame 必須是版本相符的 `Hello`（逾時 10 秒），否則終止。
- 請求依序處理；每個請求有逾時（預設 30 秒），逾時即終止 worker（`slow_requests_time_out_and_the_worker_is_stopped`）。
- 每個回應都先經 `ipc_contract::validate`；解碼失敗或驗證失敗視為協定違規，立即終止 worker。
- worker 崩潰或被終止 → 當次請求回報 `workerCrashed`，下一個請求自動啟動新的 worker；先前開啟的文件需要重新開啟（`killed_worker_is_reported_then_replaced`）。
- 文件錯誤（非 PDF、損毀、加密、頁碼錯誤）只回報錯誤，**不會**終止 worker（`worker_errors_are_reported_not_fatal`）。
- worker 的 stderr 只作診斷，持續讀取避免阻塞，不轉給前端。

## 網路

網路由三層擋住：

1. **作業系統**：AppContainer 沒有 `internetClient`、`internetClientServer`、`privateNetworkClientServer` capability，Windows 拒絕所有連線，包含 loopback。即使 MuPDF 被惡意 PDF 攻破、攻擊者自行載入 Winsock，也連不出去（`network_is_blocked_even_to_localhost`；對照組：不使用 AppContainer 時同一個 probe 可以連上）。授予任何 capability 都必須另立 ADR。
2. **連結**：worker 不連結任何網路程式庫。`cargo-deny` 禁止 HTTP／WebSocket／TLS crate（MVP-13），且測試確認 worker 執行檔的匯入表沒有 `ws2_32`、`wininet`、`winhttp`、`urlmon`、`dnsapi` 等。
3. **引擎**：MuPDF 以 `default-features = false` 建置，本身不會發出網路請求。

Job Object、restricted token 與 Low 完整性本身**不會**阻止連網；只靠它們時，同一個 probe 連得上 127.0.0.1。

## 剩餘風險與後續工作

| 風險 | 影響 | 後續 |
|---|---|---|
| AppContainer 仍可讀取授權給「所有應用程式套件」的位置（System32、Program Files、字型等） | worker 被攻破時可讀取系統檔與已安裝的程式，但讀不到使用者資料 | 評估 LPAC（需確認 MuPDF 與系統 DLL 在 LPAC 下可用）|
| 安裝檔尚未包含 `pdf_worker.exe`；worker 依賴 VC++ 執行階段（MuPDF 含 C++ 程式碼）；解除安裝尚未刪除 AppContainer profile | MVP-06 接上 UI 前需要處理 | 打包、靜態 CRT、解除安裝清理（#34） |
| 啟動時會修改 worker 執行檔的 ACL（僅新增容器 SID 的讀取＋執行） | 若安裝位置由使用者擁有，ACL 會多一筆項目 | #34 決定安裝位置時一併確認 |
| 整份文件讀入記憶體 | 大檔需要兩倍記憶體（worker 與 MuPDF 各一份） | MVP-07 評估串流讀取 |
| 請求逐一處理，`Cancel` 目前不會中斷進行中的渲染 | 取消只能等當前請求結束或逾時 | MVP-07 |

## 在本機執行測試

```bash
cargo test -p sandbox        # 以 sandbox_probe 驗證每一項限制
cargo test -p pdf_worker     # 引擎單元測試 + 真正的 worker 在沙盒中的整合測試
```
