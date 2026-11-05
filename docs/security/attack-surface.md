# 攻擊面清單（1.0 之前）

B4-11（#419）。這份清單給負責人在 1.0 之前做安全審查時對照：app 從哪些地方接受輸入、每個入口怎麼檢查、有哪些測試與 fuzz、還缺什麼。

- **範圍**：數的是 `main` 的 `da1c4c8`（2026-10-31），並確認到 `b19384e`（2026-11-03）為止，清單涉及的程式碼與設定都沒有改過。命令以 `src-tauri/src/lib.rs` 的 `generate_handler!` 為準，worker 的請求以 `crates/ipc_contract/src/worker.rs` 為準，都逐一數過。
- **整理方式**：由 AI（Claude）讀程式碼整理。AI 的整理不能取代人工安全審查（AGENTS.md）。寫「依程式碼」的項目，是讀程式碼得到的，沒有在 app 裡實際操作過。
- **怎麼用**：缺口與對應的 Issue 在 §10。1.0 之前要做的人工檢查在 [manual-checks.md](manual-checks.md) §3。
- **怎麼更新**：新增或修改命令、worker 請求、capability、寫入的位置或啟動的行程時，在同一個 PR 更新對應的列。CI 會自動檢查一致性（見下方 CI 一致性檢查說明）。
- **CI 一致性檢查**（#430）：由 `scripts/ci/check-security-config.mjs` 自動比對這份清單與程式碼：
  - `src-tauri/src/lib.rs` 的 `generate_handler!` 裡的每個命令，必須與 §1 各小節表格第一欄完全一致（雙向比對）；
  - `crates/ipc_contract/src/worker.rs` 的 `WorkerRequest` 變體，必須與 §2.4 表格第一欄完全一致（雙向比對）；
  - `crates/ipc_contract/src/worker.rs` 的 `WorkerEdit` 變體，必須與 §2.5 表格第一欄完全一致（雙向比對）；
  - `src-tauri/capabilities/main.json` 的每個 `allow-<名稱>` 權限必須對應 §1 的命令；非 `allow-*` 的權限必須出現在 §4；
  - 各節與小節標題中的數量標記（如 `（88 個）`、`（46 個）`、`（26 種）`、`（11 個）`）必須等於表格中的項目數，§1 大標題的數量必須等於底下各小節命令總數。
  - **失敗時怎麼修**：
    - 若新增或刪除命令：在 §1 對應的小節表格加列或刪列，並同步更新該小節標題與 §1 大標題的數量標記；
    - 若新增或修改 worker 請求或編輯：在 §2.4 或 §2.5 的表格加列或刪列，並更新標題的數量標記；
    - 若新增 capability 權限：若是命令權限（`allow-*`）確認命令已在 §1，其他權限確認已寫入 §4；
    - 修改後在本機執行 `node scripts/ci/check-security-config.mjs` 與 `node --test scripts/ci/*.test.mjs` 確認通過。

相關文件：[worker-sandbox.md](../architecture/worker-sandbox.md)、[ipc-contract.md](../architecture/ipc-contract.md)、[local-data.md](../architecture/local-data.md)、[offline-verification.md](offline-verification.md)、[fuzzing.md](fuzzing.md)、[ADR 0008](../adr/0008-pdf-worker-isolation.md)、[ADR 0009](../adr/0009-default-network-policy.md)。

## 信任邊界

| 邊界 | 不可信的一方 | 通道 | 主要的防線 |
|---|---|---|---|
| WebView → 主行程 | 前端頁面（假設可能被注入程式碼） | Tauri IPC（`ipc:`、`http://ipc.localhost`） | capability 只開放 app 自己的 89 個命令（§4）；每個命令檢查參數（§1）；前端只拿得到代號，拿不到路徑 |
| worker → 主行程 | `pdf_worker`（假設已被惡意檔案控制） | worker 標準輸入、輸出上的 postcard frame，每個 ≤ `MAX_FRAME_BYTES`（80 MiB） | 沙盒（§2.1）；每個回應都先驗證（§2.2）；逾時 |
| 檔案 → worker | PDF 與其他要讀的檔案 | 主行程開好的唯讀檔案代號，或主行程讀好的位元組 | 沙盒；程式庫見 §2.3 |
| 檔案 → 主行程 | app 資料夾的檔案；使用者匯入的語言資料、字型、`.pfx` | 主行程自己讀 | 大小上限與格式檢查（§3） |
| 網路 → 主行程 | 檢查更新的回覆 | WinHTTP，只連一個固定的網址（§5） | ≤ 1 MiB，只讀版本號 |
| 其他程式 → 主行程 | 同一個使用者的其他程式 | 命令列、第二次啟動、拖放、環境變數（§8） | 只接受檔案路徑，開檔和對話框走同一條路 |

以下各節都以這四條規則為前提：

1. 前端不傳也不收檔案路徑或 URI。路徑只在主行程與 Windows 的對話框之間；前端拿到的是文件、分頁、附件、連結、最近的檔案、字型、憑證的代號或編號（ADR 0008）。
2. worker 不能自己開任何路徑。要讀的檔案是唯讀代號（`FILE_GENERIC_READ`），要寫的檔案是只能寫的代號（`FILE_WRITE_DATA | FILE_APPEND_DATA | SYNCHRONIZE`），都由主行程開好再交給它。
3. 寫到使用者的檔案時，先寫同一個資料夾裡新建的暫存檔，寫完（存檔還要再檢查）才改名取代。批次只寫新的檔名，不取代任何檔案（§6）。
4. 只有 `src-tauri/src/update_check.rs` 會連網，而且只在使用者按「檢查更新」時（ADR 0009，§5）。

## 1. 前端能呼叫的命令（89 個）

- **數目一致**：`generate_handler!`、`src-tauri/build.rs` 的 `AppManifest::commands`、`src-tauri/capabilities/main.json` 的 `allow-*` 都是同樣的 89 個（整理時以 diff 確認）。CI 的 `scripts/ci/check-security-config.mjs` 檢查前兩者一致，也檢查 capability 只用它允許清單裡的權限。每個權限的用途寫在那份允許清單裡，下表不再重複。
- **自由文字**：沒有命令接受路徑或 URI。會帶自由文字的只有這些參數：密碼、剪貼簿的文字、搜尋字串、個資的關鍵字、註解與文字方塊的文字、表單欄位的值、書籤標題、浮水印與頁面標記的文字、簽章的理由、設定、OCR 語言代碼。
- **參數檢查**：參數先由 serde 解碼成固定的型別，型別不對就拒絕；再由 `crates/ipc_contract/src/validate.rs` 檢查（表中寫成 `XxxArgs`）。上限都在 `crates/ipc_contract/src/limits.rs`。主行程沒有另外限制一個 IPC 訊息的大小，參數整個解碼後才檢查（§9）。
- **對話框**：都是 Windows 的 `IFileDialog`（`src-tauri/src/file_dialog.rs`），一律加 `FOS_FORCEFILESYSTEM | FOS_DONTADDTORECENT`，只能選真的檔案，也不記入 Windows 的最近使用。另存的對話框再加 `FOS_OVERWRITEPROMPT`。
- **選到文件自己的檔案時**：寫出副本、匯出的命令都會說明不能取代，再問一次（`is_document_file`、`tell_same_file`）。例外是匯出純文字（#423）。
- **測試**：`commands.rs` 沒有單元測試。命令由 E2E 測（`tests/e2e/*.spec.ts`，操作真正的 app），背後的邏輯由各模組的單元測試測。下表寫 spec 的檔名或模組；「worker」指 `crates/pdf_worker/tests/` 的整合測試。

### 1.1 開檔與分頁（11 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `subscribe_open_events` | 用 channel 接收開檔的結果（不開放 `core:event`） | 沒有參數 | — | `open.spec.ts` |
| `open_document_dialog` | 主行程顯示開啟對話框，選到的每個 PDF 各開一個分頁 | 同時只開一個（`DIALOG_SHOWING`）；分頁 ≤ `MAX_TABS`（20）；檔案 ≤ `MAX_DOCUMENT_BYTES`（512 MiB） | 開啟對話框（`*.pdf`，可多選）；每個分頁啟動一個 worker；記入最近的檔案（§6） | `open.spec.ts`、`tabs.spec.ts`、`unicode-paths.spec.ts` |
| `retry_open` | worker 失敗後，以分頁代號重新開啟那個分頁的檔案 | 分頁代號 | 新的 worker | `retry-open-and-cancel.spec.ts`；worker `isolation.rs`（`killed_worker_is_reported_then_replaced`） |
| `unlock_tab` | 對加密文件的分頁試密碼；使用者要求時記住 | `UnlockArgs`：`check_password`（不得為空、不含 NUL、≤ `MAX_PASSWORD_BYTES` 1024 bytes） | 密碼只送到那個分頁的 worker；要記住時寫入認證管理員（§6） | `password.spec.ts`、`password-memory.spec.ts`；`credentials.rs` |
| `forget_password` | 忘記某個分頁的檔案記住的密碼 | 分頁代號 | 刪除認證管理員裡的那一筆 | `password-memory.spec.ts` |
| `forget_all_passwords` | 忘記全部記住的密碼，回傳原本有幾筆 | 沒有參數 | 刪除認證管理員裡 app 的全部項目 | `password-memory.spec.ts` |
| `get_remembered_passwords` | 記住了幾個密碼、最多能記幾個 | 沒有參數；不回傳清單、鍵或密碼 | — | `password-memory.spec.ts` |
| `pick_pages_source` | 選一個 PDF，把它的頁面插入文件（B2-06） | 先 `check_can_insert_pages` 再開對話框；檔案 ≤ `MAX_SOURCE_BYTES`（64 MiB） | 開啟對話框；文件的 worker 讀它，做成乾淨的副本（`PrepareSource`） | `merge.spec.ts`；worker `merge.rs` |
| `unlock_pages_source` | 剛選的來源檔要密碼時試密碼 | `UnlockSourceArgs`：`check_password` | 只送到那份文件的 worker | `merge.spec.ts` |
| `close_tab` | 關閉分頁，結束它的 worker | 分頁代號 | 清掉文字比較的快取 | `tabs.spec.ts` |
| `set_active_tab` | 視窗顯示哪個分頁（給視窗標題用） | 分頁代號 | — | `tabs.spec.ts` |

### 1.2 閱讀（13 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `render_page` | 渲染一頁，或一頁的一塊 | `RenderPageArgs`：`check_scale`（0.01–64）、`check_page_index`；區塊每邊 ≤ 8192 px、總像素 ≤ 4096²、座標在 2²⁰ 以內 | `Render`、`RenderRegion` | `viewer.spec.ts`、`thumbnails.spec.ts`；`documents.rs`（`regions_render_at_the_requested_scale`）；worker `isolation.rs`（`memory_limit_contains_a_huge_render`） |
| `cancel` | 以請求代號取消排隊或進行中的工作（渲染、搜尋、匯出、批次、索引搜尋） | 請求代號 | 只在主行程裡取消；worker 的 `Cancel` 目前沒有送出（§2） | `retry-open-and-cancel.spec.ts`；前端的單元測試 |
| `get_outline` | 讀目錄（已檢查過） | 文件代號 | `GetOutline` | `outline-edit.spec.ts`；worker `outline.rs` |
| `get_page_text` | 一頁的文字與每個字的位置（選取、複製） | `check_page_index` | `GetPageText` | `text.spec.ts`；worker `text.rs` |
| `get_page_links` | 一頁的連結在哪裡、是哪一種 | `check_page_index` | `GetPageLinks` | worker `links.rs`；`links.rs` |
| `get_page_annotations` | 一頁的註解（不含連結、表單欄位與彈出視窗） | `check_page_index` | `GetPageAnnotations` | `annotations.spec.ts`、`ink-stamps.spec.ts` |
| `get_page_fields` | 一頁的表單欄位（值、選項、有沒有腳本；腳本一律不執行） | `check_page_index` | `GetPageFields` | `forms.spec.ts` |
| `get_signatures` | 離線驗證文件的簽章 | 文件代號 | `VerifySignatures` | `signatures.spec.ts`；worker `signatures.rs` |
| `get_redaction_summary` | 套用遮蔽會動到多少東西（只有數量） | 文件代號 | `SummariseRedactions` | `redaction.spec.ts` |
| `find_personal_data` | 找個資，回傳數量與遮掉一部分的預覽 | `FindPersonalDataQuery`：類別 ≤ 10 個；關鍵字只能配它的類別，不得空白，≤ 1024 bytes | `FindPersonalData` | `redaction.spec.ts`；worker `personal_data.rs` |
| `get_attachments` | 附件清單（清理過的名稱、宣稱的大小、類型是否危險） | 文件代號 | `GetAttachments` | `attachments.spec.ts`；worker `attachments.rs` |
| `save_attachment` | 把一個附件存到使用者選的地方；app 從不開啟附件（ADR 0025） | 附件代號必須在清單裡；目的地不得是文件自己的檔案（`same_file`），而且要可寫（`check_writable`） | 另存對話框（附件的副檔名，加 `*.*`）；worker 以只能寫的代號寫到暫存檔，再改名；加上 `Zone.Identifier`（來自網際網路） | `attachments.spec.ts`；`documents.rs`（`an_attachment_is_saved_as_it_is_and_marked_as_from_the_internet`）；`attachments.rs` |
| `set_fullscreen` | 設定視窗進出全螢幕（B4-03） | `on: bool` | — | `presentation.spec.ts` |

### 1.3 連結、外部網址與檢查更新（10 個）

開啟連結分兩步：先由 `describe_*` 取得確認對話框要顯示的內容，使用者確認後才呼叫 `open_*`。兩步都在主行程經過 `src-tauri/src/links.rs` 的 `links::preview`：只允許 `http`、`https`、`mailto`（遠端參照只允許 `http`、`https`），網址轉成 ASCII（網域用 punycode）再顯示與開啟。前端只能以編號指定連結，不能給 URI。

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `describe_link`、`open_link` | 頁面上的連結 | 以文件、頁碼與連結編號指定；`links::preview` | `open_link`：`ShellExecuteW`（§7） | `links.rs`（`the_frontend_can_only_name_a_link_never_give_a_uri` 等 6 個）；人工檢查 manual-checks §2 |
| `describe_outline_link`、`open_outline_link` | 目錄項目的連結 | 以目錄項目的位置指定；同上 | 同上 | 同上 |
| `describe_remote_address`、`open_remote_address` | 遠端參照的網址（B3-08）交給瀏覽器 | 以位置指定；只允許 `http`、`https` | 同上 | `remote-references.spec.ts` |
| `describe_releases_page`、`open_releases_page` | 固定的發行頁面 | 網址是常數，也經過 `links::preview` | 同上 | `settings.spec.ts` |
| `open_default_apps_settings` | 開啟 Windows 設定的「預設應用程式」頁 | 常數 `ms-settings:defaultapps?registeredAppMachine=PDF%20Reader` | `ShellExecuteW` | 前端的單元測試；人工檢查（manual-checks §3） |
| `check_for_updates` | 唯一的網路請求（§5） | 沒有參數；同時只跑一個 | WinHTTP | `update_check.rs` 6 個；`settings.spec.ts` |

### 1.4 搜尋與文字索引（5 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `search` | 全文搜尋，結果送到 channel | `SearchArgs`：不得為空，≤ `MAX_QUERY_BYTES`（1024 bytes） | 主行程逐頁送 `SearchPage` | `search-bar.spec.ts`；worker `search_cache.rs` |
| `search_text_index` | 搜尋之前開過的文件的文字索引（ADR 0026）；結果只有檔名、頁碼與一小段文字 | `IndexSearchArgs`：同上 | 讀 app 資料夾的 `index\`（§3） | `text-index.spec.ts`；`text_index.rs` 24 個 |
| `open_indexed_document` | 以編號開啟上次索引搜尋的結果 | 索引要開著；分頁 ≤ 20；主行程確認檔案沒有改過 | 開檔，同 `open_document_dialog` | `text-index.spec.ts` |
| `get_text_index_status` | 索引有幾份文件、佔多少空間 | 沒有參數 | — | `text-index.spec.ts` |
| `clear_text_index` | 刪除索引，先刪金鑰 | 沒有參數 | 刪除 `index\` | `text-index.spec.ts`；`text_index.rs` |

### 1.5 最近的檔案與設定（9 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `get_recent_files` | 最近的檔案（名稱與代號，沒有路徑） | 沒有參數；≤ `MAX_RECENT_FILES`（20） | 讀 `recent.json` | `recent.spec.ts`；`recent.rs` |
| `open_recent_file` | 以代號開啟 | 檔案要還在 | 開檔 | `recent.spec.ts` |
| `remove_recent_file`、`clear_recent_files` | 從清單拿掉一筆，或清空 | 代號 | 寫 `recent.json` | `recent.spec.ts` |
| `get_file_recording`、`set_file_recording` | 這份文件的檔案要不要記在清單上 | 文件代號；不記錄的檔案只存雜湊，要用隨機值，取不到就拒絕 | 寫 `recent.json` | `recent.spec.ts`；`recent.rs` |
| `clear_recent_exclusions` | 忘記哪些檔案不要記錄 | 沒有參數 | 寫 `recent.json` | `recent.rs` |
| `get_settings` | 讀設定 | 沒有參數 | 讀 `settings.json` | `settings.spec.ts`；`settings.rs` |
| `set_settings` | 取代設定 | `Settings`：不接受未知的欄位；OCR 語言必須是語言代碼（`is_language_name`） | 寫 `settings.json`；關掉「記錄最近的檔案」時，清空清單、復原日誌與文字索引 | `settings.spec.ts`；`settings.rs` |

### 1.6 機敏文件（5 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `check_sensitive_mark` | 寫入機敏標記會怎麼樣（ADR 0010） | 文件代號 | — | `sensitive.spec.ts` |
| `set_sensitive` | 使用者確認後，在文件自己的檔案加上或拿掉機敏標記 | `SensitiveArgs`：加密文件的密碼要過 `check_password` | worker 以增量更新寫到暫存檔（`WriteIdentifier`），再取代文件的檔案；更新 `sensitive.json` | `sensitive.spec.ts`；worker `sensitive.rs`；`sensitive.rs` |
| `get_sensitive_files` | 標成機敏的檔案（名稱與代號） | 沒有參數 | 讀 `sensitive.json` | `sensitive.spec.ts` |
| `remove_sensitive_file` | 從清單拿掉一筆（檔案本身不變） | 代號 | 寫 `sensitive.json` | `sensitive.spec.ts` |
| `copy_protected_text` | 把機敏文件的文字放到剪貼簿，不進剪貼簿歷程（ADR 0004） | `check_clipboard_text`：不得為空、≤ 16 MiB、不含 NUL | 寫剪貼簿，加上不進歷程的格式；從不讀剪貼簿 | `sensitive.spec.ts`、`clipboard.spec.ts`；`clipboard.rs` |

### 1.7 編輯與存檔（9 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `apply_edit` | 一個有型別的編輯，在文件的 worker 套用（ADR 0013） | `EditArgs`：每種編輯各自的檢查（`Edit::validate`，§2.4）；作者禁止註解時拒絕；undo ≤ `MAX_UNDO_EDITS`（1000） | `Edit`；寫復原日誌（§6） | `annotations.spec.ts`、`pages.spec.ts`、`forms.spec.ts`、`redaction.spec.ts`、`outline-edit.spec.ts` 等；`documents.rs`（`an_author_who_forbids_annotations_is_obeyed` 等） |
| `undo_edit` | 復原上一個編輯：worker 從保留的位元組重新開啟，再做一次其餘的編輯 | `UndoArgs`：以密碼開啟的文件要再給密碼，過 `check_password` | `Revert` | `saving.spec.ts` 等；worker `saving.rs` |
| `redo_edit` | 重做上一個復原的編輯 | 文件代號 | `Edit` | 同上 |
| `recover_edits` | 套用上次執行留下的復原日誌（當機復原，B2-13） | 文件代號；日誌當成不可信的資料檢查（§3） | 讀 `recovery\` | `recovery.spec.ts`；`recovery.rs` 14 個 |
| `discard_recovered_edits` | 刪除那份復原日誌 | 文件代號 | 刪除 `recovery\` 裡的檔案 | `recovery.spec.ts` |
| `save_document` | 寫回文件自己的檔案 | 路徑只在主行程 | worker 以只能寫的代號寫到同一個資料夾的暫存檔，主行程檢查後取代；之後 `Rebase` | `saving.spec.ts`；`saving.rs` 11 個；worker `saving.rs` |
| `save_document_as` | 另存新檔 | 文件代號 | 另存對話框（`*.pdf`）；其餘同上 | `saving.spec.ts`、`unicode-paths.spec.ts` |
| `close_window` | 使用者回答未存的變更之後，關閉視窗 | `discard` 是 false 而且還有未存的變更時，拒絕並請頁面詢問使用者 | 批次還在跑時只隱藏視窗（通知區圖示留著）；否則丟掉未存的變更，關閉視窗 | `saving.spec.ts`、`batch.spec.ts` |
| `pick_stamp_image` | 選一張圖片做自訂圖章（B2-08） | 先開對話框，之後才檢查文件（§9） | 開啟對話框（`*.png;*.jpg;*.jpeg`）；文件的 worker 讀它，只留像素（`PrepareStampImage`） | `ink-stamps.spec.ts`；worker `stamp_image.rs` |

### 1.8 匯出、比較與副本（9 個）

寫出副本的命令都先由主行程問使用者存到哪裡，選到文件自己的檔案就說明並再問一次，然後由文件的 worker 以只能寫的代號寫到暫存檔，主行程再改名。原本的文件不變。

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `export_pages` | 匯出頁面：純文字、Word、PNG、JPEG、PDF（拆分） | `ExportArgs`：頁面不得為空、不重複、≤ 1000（PDF ≤ 100,000）；dpi 只能是 72、150、300；每 N 頁一個檔時 N > 0、≤ 1000 個檔；作者禁止複製時拒絕；加密文件不能拆成 PDF | 純文字、Word、PDF：另存對話框，擋文件自己的檔案（#423）。圖片、每 N 頁一個檔：選資料夾，已有的檔案要確認取代。`RenderPng`、`RenderJpeg`、`ExportDocx`、`SavePages` | `export.spec.ts`（含純文字不能取代原檔）、`split.spec.ts`；`export.rs` 9 個；worker `docx.rs` |
| `export_annotation_summary` | 註解摘要寫成純文字或 Markdown | 作者禁止複製時拒絕 | 另存對話框，擋文件自己的檔案；`AnnotationSummary` | `annotation-summary.spec.ts`；`export.rs` |
| `compare_text` | 比較兩份開著的文件的文字 | `CompareTextArgs`：不能和自己比；任一份的作者禁止複製時拒絕；期限 ≤ `MAX_COMPARE_DEADLINE_MILLIS`（10 秒） | 讀兩份文件的頁面文字 | `compare.spec.ts`；`src-tauri/src/compare/` |
| `export_compare_text` | 比較結果寫成純文字或 Markdown | `ExportCompareTextArgs`：同上 | 另存對話框，兩份文件自己的檔案都擋 | `compare.spec.ts` |
| `privacy_export` | 不含中繼資料的副本（B2-03） | 加密文件拒絕 | 另存對話框；`PrivacyCopy`（識別碼由主行程隨機產生） | `privacy-export.spec.ts` |
| `sanitize_copy` | 去除主動內容、附件與隱藏資料的副本（B3-15） | `SanitizeArgs`；加密文件拒絕，已簽章的文件由 worker 拒絕 | 另存對話框；`SanitizedCopy`，寫出前先做主動內容掃描 | `sanitize.spec.ts`；worker `sanitize.rs` |
| `page_marks_copy` | 加上頁碼、Bates 編號、頁首頁尾的副本（B3-16） | `PageMarksCopyArgs`：字級與邊界在範圍內；頁面遞增；文字是一行、不含控制字元與看不見的格式字元；位置不重複；Bates 的位數與起始值不溢位；加密文件拒絕 | 另存對話框；`PageMarksCopy` | `page-marks.spec.ts`；worker `page_marks.rs` |
| `compress_images` | 圖片重新編碼、縮小的副本（B3-01） | `CompressArgs`：長邊 256–16,384 px；加密文件拒絕，已簽章的文件由 worker 拒絕 | 另存對話框；`CompressedCopy` | `compress-images.spec.ts`；worker `compress.rs`；fuzz `compress_images` |
| `encrypt_copy` | AES-256 加密的副本（B2-15） | `EncryptArgs`：新密碼 ≤ `MAX_NEW_PASSWORD_BYTES`（127 bytes）；要有開啟密碼或限制；有限制就要有權限密碼；兩個密碼不得相同；加密文件拒絕，已簽章的文件由 worker 拒絕 | 另存對話框；`EncryptedCopy`。沒有給權限密碼時，主行程產生 128 位元的隨機密碼；密碼用完即清除 | `encrypt.spec.ts`；worker `encrypt_copy.rs` |

### 1.9 批次與建立 PDF（2 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `run_batch` | 對許多檔案做同一件事：去除中繼資料、去除 EXIF、加密、浮水印、壓縮圖片、清理、頁面標記（B3-04、ADR 0024）。原檔不變 | `BatchArgs`：各操作的檢查同 §1.8 的副本；檔案 ≤ `MAX_BATCH_FILES`（500） | 浮水印圖片的開啟對話框（新的 worker 做 `PrepareStampImage`）；開啟對話框（可多選）；選資料夾。每個檔案一個新的 worker；副本用不重複的新檔名，報告以 `create_new` 建立；在背景執行，通知區有圖示 | `batch.spec.ts`；`documents.rs`（`a_batch_compresses_the_pictures_of_each_file_and_refuses_a_signed_one` 等） |
| `create_pdf` | 把圖片、網頁、純文字、EPUB、XPS、SVG、Office 檔做成 PDF（B3-03） | 同時只跑一個（`CREATING`）；來源 ≤ `MAX_CREATE_SOURCES`（200）、每個 ≤ 64 MiB；不得存到來源檔上（`same_file`） | 開啟對話框（可多選）；另存對話框；新的 worker 以唯讀代號讀來源（`CreatePdf`） | `create-pdf.spec.ts`；worker `create_pdf.rs`；`documents.rs`；fuzz `create_pdf` |

### 1.10 簽章（4 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `check_signing` | 文件能不能簽 | 文件代號 | — | `signing.spec.ts`；`documents.rs`（`what_cannot_be_signed_is_said_before_the_user_is_asked_anything`） |
| `choose_signing_file` | 選一個 PKCS #12 檔（B3-06） | 檔案 ≤ `MAX_CERTIFICATE_FILE_BYTES`（1 MiB） | 開啟對話框（`*.pfx;*.p12`）；位元組留在主行程的記憶體，前端只拿到編號與檔名 | `signing.spec.ts` |
| `list_signing_certificates` | 使用者「個人」存放區裡有私密金鑰的憑證（唯讀，ADR 0020） | ≤ `MAX_STORE_CERTIFICATES`（100） | 不開啟金鑰 | `signing.spec.ts` |
| `sign_document` | 做一份簽章的副本 | `SignArgs`：憑證檔的密碼過 `check_password`；理由 ≤ 256 bytes、只有一行；時區在 ±14 小時內；憑證檔必須是上次選的那個，存放區的憑證必須是上次列出的 | 另存對話框，擋文件自己的檔案。PFX 由 Windows 匯入（§3）；worker 加上簽章欄位（`PrepareSignature`）；主行程寫入 CMS；新的 worker 驗證這份副本（`VerifySignatures`），再開成新分頁。智慧卡由 Windows 問 PIN | `signing.spec.ts`；`signing.rs` 11 個；`documents.rs`（`a_signed_copy_verifies_and_one_changed_byte_breaks_it` 等） |

### 1.11 字型與文字方塊（6 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `get_fonts` | Windows 已安裝的字型與匯入的字型（代號與名稱，沒有路徑） | 沒有參數 | 讀字型清單 | `fonts.spec.ts`、`text-box.spec.ts`；`fonts.rs` 14 個 |
| `prepare_text_box` | 把文字方塊的字做成字型的小子集（ADR 0018） | `PrepareTextBoxArgs`：`check_text_box_text`（不得空白、≤ 4096 bytes、`is_note_text`）；作者禁止註解時拒絕 | 主行程讀字型檔（≤ 64 MiB）送給 worker（`PrepareTextBox`） | `text-box.spec.ts`；worker `text_box_worker.rs`；fuzz `text_box_font` |
| `get_text_box` | 文字方塊的字、字型名稱與樣式 | `check_page_index` | `GetTextBox` | `text-box.spec.ts`；worker `text_box_worker.rs` |
| `import_font` | 匯入 `.ttf`、`.ttc`（B2-32） | 同時只開一個（`FONT_DIALOG_SHOWING`）；副檔名、大小（≤ 64 MiB）、檔頭（`check_import_header`）；再由新的 worker 檢查 TrueType（`InspectFont`）；匯入 ≤ `MAX_IMPORTED_FONTS`（50） | 開啟對話框；複製到 app 資料夾的 `fonts\`，更新 `metadata.json` | `fonts.spec.ts`；`fonts.rs`（`import_header_checks`、`import_refuses_bad_extension_unreadable_and_too_large`） |
| `remove_font` | 以代號移除匯入的字型 | `RemoveFontArgs` | 刪除 `fonts\` 裡的檔案 | `fonts.rs` |
| `remove_all_fonts` | 移除全部匯入的字型；Windows 的字型不動 | 沒有參數 | 同上 | `fonts.rs` |

### 1.12 OCR（6 個）

| 命令 | 做什麼 | 主行程的檢查與上限 | 對話框、寫入、行程 | 測試 |
|---|---|---|---|---|
| `get_ocr_languages` | 可用的語言（代碼與大小） | 沒有參數 | 讀內附的 `tessdata\` 與 app 資料夾的 `tessdata\` | `ocr.spec.ts`；`ocr_languages.rs` 7 個 |
| `import_ocr_language` | 匯入 `.traineddata`（B2-10、ADR 0015） | 同時只開一個（`LANGUAGE_DIALOG_SHOWING`）；名稱過 `is_language_name`，大小 ≤ 64 MiB，內容過 `check_language_data`；匯入 ≤ `MAX_IMPORTED_LANGUAGES`（20） | 開啟對話框；複製到 app 資料夾的 `tessdata\` | `ocr.spec.ts`；`crates/ipc_contract/src/ocr.rs` 8 個 |
| `remove_ocr_language` | 以代碼移除匯入的語言；內附的不能移除 | `RemoveLanguageArgs`：`is_language_name`，路徑字元進不來 | 刪除檔案 | `ocr.spec.ts`；`ocr_languages.rs` |
| `start_ocr` | 現在開始辨識文件的掃描頁 | 文件代號 | 主行程讀語言資料送給文件的 worker（`OcrLoad`），再逐頁 `OcrPage`、`OcrPoll` | `ocr.spec.ts`；`src-tauri/src/ocr.rs` 14 個；worker `ocr.rs` |
| `stop_ocr` | 停止辨識 | 文件代號 | `OcrStop` | `src-tauri/src/ocr.rs`；worker `ocr.rs` |
| `set_ocr_focus` | 先辨識使用者正在看的那一頁 | `check_page_index` | — | `src-tauri/src/ocr.rs` |

## 2. worker

### 2.1 沙盒

`pdf_worker.exe` 只由 `crates/sandbox` 的 `Sandboxed::spawn`（`CreateProcessAsUserW`）啟動，細節見 [worker-sandbox.md](../architecture/worker-sandbox.md) 與 ADR 0008。

| 限制 | 內容 | 測試（`crates/sandbox/tests/sandbox.rs`，另註明的除外） |
|---|---|---|
| Job Object | 關閉時結束、只能有 1 個行程、記憶體 ≤ 2 GiB、未處理的例外直接結束、UI 限制 | `memory_limit_is_enforced`、`peak_memory_is_reported`、`kill_terminates_the_process`、`dropping_the_handle_kills_the_process`、`cannot_start_child_processes` |
| 權杖 | restricted token（`DISABLE_MAX_PRIVILEGE`）、Low integrity | `runs_at_low_integrity`；worker `isolation.rs`（`worker_runs_at_low_integrity_in_an_app_container`） |
| AppContainer | `PdfReader.Worker`，沒有任何 capability：不能連網，讀不到使用者的檔案 | `network_is_blocked_even_to_localhost`、`cannot_read_user_files`、`cannot_write_to_user_locations`、`root_certificates_are_readable`、`system_fonts_are_readable`、`app_container_profile_can_be_deleted`；`installer_hooks.rs`（`uninstaller_deletes_the_worker_profile`） |
| 行程的緩解措施 | 停用 win32k、ACG、ASLR、嚴格的代號檢查、不載入遠端與 Low integrity 位置的映像、優先 System32、停用舊的擴充點、`CHILD_PROCESS_RESTRICTED` | `win32k_is_unavailable`、`cannot_start_child_processes` |
| 繼承的東西 | 只有 3 個 pipe 代號；環境變數只有 `SystemRoot` 與 `LOCALAPPDATA`；`DETACHED_PROCESS`；工作目錄是 System32 | `pipes_carry_stdin_and_stdout`、`environment_is_minimal` |
| 檔案代號 | 讀的只能讀，寫的只能寫 | `duplicated_file_handles_are_read_only`、`duplicated_write_handles_are_write_only` |
| 執行檔 | 不匯入任何網路 API | worker `isolation.rs`（`worker_binary_imports_no_networking`） |

已接受的限制（§9）：不是 LPAC；worker 的執行檔放在使用者擁有的位置時（例如開發建置），主行程會改它的 ACL，讓 AppContainer 能執行它。

### 2.2 主行程怎麼對待 worker 的回應

- 每個 frame 先以 postcard 解碼，再經 `WorkerResponse::validate`（`crates/ipc_contract/src/validate.rs`，涵蓋每一種回應）。解碼失敗、驗證失敗、第一個 frame 不是相符的 `Hello`：停止 worker，回報 `protocolViolation`，下一個請求啟動新的 worker（`crates/worker_host/src/lib.rs`）。
- 期限：握手 10 秒，一般請求 30 秒，存檔類 5 分鐘（`SAVE_TIMEOUT`）。worker 當掉、逾時、違反協定，都當成這份文件的 worker 遺失。
- 文件的檔案 ≤ `MAX_DOCUMENT_BYTES`（512 MiB），在交給 worker 之前檢查。
- 處理缺口（已在 #422 修正）：
  - 期限從送出開始算，延遲或晚到的回應不會重新計時；
  - 編號不相符（含 0、未來編號或過去編號）或種類不符（`is_expected_response`、`unexpected_on`）時視為協定違規，停止 worker 並將文件標記為 lost；
  - `worker_host/tests/replies.rs` 以 `fake_worker` 完整測試協定違規（無效 frame、驗證失敗、編號 0、未來編號、延遲送不相符編號、種類不符、重複 Hello、逾時）會停止 worker 且下一個請求啟動新 worker 成功。
- 測試：`crates/ipc_contract` 的驗證測試；worker `isolation.rs`（`worker_errors_are_reported_not_fatal`、`killed_worker_is_reported_then_replaced`、`slow_requests_time_out_and_the_worker_is_stopped`）；`documents.rs`（`test_prepare_inserted_text_boxes_crashed_worker_marks_document_lost`）。fuzz `worker_messages` 確認兩個方向的解碼與驗證不會 panic。

### 2.3 worker 裡解析不可信輸入的程式庫

| 程式庫 | 解析什麼 | `unsafe` | fuzz |
|---|---|---|---|
| MuPDF 1.27.2（`mupdf` =0.8.0、`mupdf-sys` =0.8.0） | PDF（含內嵌的字型與圖片）、PNG、JPEG、EPUB、HTML、SVG、XPS、docx、xlsx、pptx、純文字 | C 程式庫；`owner_password.rs` 直接呼叫 C 函式 | `open_document`、`create_pdf`、`compress_images`、`exif_clean`、`stamp_image`；缺的見 #426、#427 |
| Tesseract、Leptonica（MuPDF 內建的 Artifex 版本） | `.traineddata`、掃描頁的像素 | C++、C；呼叫集中在 `ocr.rs` | 沒有（#431） |
| skrifa 0.49、subsetter 0.2.6 | 文字方塊與匯入用的字型 | 純 Rust，沒有 `unsafe` | `text_box_font`（含 `InspectFont`，#428） |
| Windows CryptoAPI（`crypt.rs`） | 簽章的 CMS 與憑證；憑證鏈只用本機的快取 | Windows | 沒有：Linux 的 fuzz 跑不到（#426 只 fuzz 讀簽章欄位） |
| jpeg-encoder 0.7.1 | 只編碼 worker 自己渲染的像素，不解析輸入 | 純 Rust（不開 `simd`） | 不需要 |

### 2.4 worker 的請求（46 個）

除了另外註明的，請求都送到那個分頁自己的 worker（ADR 0012）。用新的 worker 的有：驗證剛簽好的副本、批次的每個檔案、`CreatePdf`、`InspectFont`、批次的浮水印圖片。

| 請求 | 讀或寫什麼 | 程式碼（`crates/pdf_worker/src/`） | 測試 | fuzz |
|---|---|---|---|---|
| `Open` | PDF（唯讀代號）與可能的密碼；開啟時掃描主動內容 | `engine.rs`、`scan.rs`、`owner_password.rs` | worker `isolation.rs`、`encryption.rs`、`corpus_crypto.rs`、`active_content.rs`、`password_key.rs` | `open_document`（無密碼）；`read_document`（帶密碼路徑，#426） |
| `Render`、`RenderRegion` | 頁面的內容 | `engine.rs` | worker `isolation.rs`；`viewer.spec.ts` | `open_document` |
| `RenderPng`、`RenderJpeg` | 同上，再編成 PNG 或 JPEG | `engine.rs`、`jpeg.rs` | `export.spec.ts`；`jpeg.rs` | 渲染的部分：`open_document` |
| `GetOutline` | 目錄 | `engine/outline.rs` | worker `outline.rs` | `open_document` |
| `GetPageLinks` | 連結與它們的 URI | `engine.rs` | worker `links.rs` | `open_document` |
| `GetPageAnnotations` | 註解 | `engine/annotations.rs` | worker `merge.rs`、`stamp_image.rs`、`text_box_worker.rs`；`annotations.spec.ts` | `read_document`（前兩頁，#426） |
| `GetPageFields` | 表單欄位 | `engine/forms.rs`（沒有單元測試） | `forms.spec.ts` | `read_document`（前兩頁，#426） |
| `VerifySignatures` | 簽章欄位、簽章字典、CMS | `engine/signatures.rs`、`signatures.rs`、`crypt.rs` | worker `signatures.rs`、`signature_poc.rs`；`signatures.spec.ts` | `read_document`（簽章欄位部分，#426） |
| `SummariseRedactions` | 遮蔽標記會蓋到的東西 | `engine/redaction.rs` | `redaction.rs` 的單元測試；`redaction.spec.ts` | `read_document`（#426） |
| `FindPersonalData` | 頁面文字 | `engine/personal_data.rs` | worker `personal_data.rs` | `read_document`（#426） |
| `GetPageText` | 頁面文字 | `engine.rs`、`text_cache.rs` | worker `text.rs` | `read_document`（前兩頁，#426） |
| `SearchPage` | 頁面文字 | `search.rs` | worker `search_cache.rs` | `open_document`（前兩頁） |
| `AnnotationSummary` | 每一頁的註解，與註解蓋住的文字 | `engine.rs` | `engine.rs` 的單元測試；`annotation-summary.spec.ts` | `read_document`（#426） |
| `GetAttachments` | 附件清單 | `engine/attachments.rs`（沒有單元測試） | worker `attachments.rs` | `read_document`（#426） |
| `WriteAttachment` | 解碼附件（≤ 512 MiB），寫到只能寫的代號 | `engine/attachments.rs` | worker `attachments.rs` | `read_document`（至多前 2 個，上限 1 MiB，#426） |
| `GetTextBox` | 文字方塊註解 | `engine/text_box.rs` | worker `text_box_worker.rs` | `read_document`（前兩頁，#426） |
| `ExportDocx` | 頁面的文字與圖片，寫成 .docx | `engine/docx.rs` | worker `docx.rs` | `open_document` |
| `Edit` | 29 種編輯（§2.5） | `engine.rs` 等 | worker `saving.rs`、`merge.rs`、`outline_edit.rs`、`stamp_image.rs`、`text_box_worker.rs` | 沒有（#427） |
| `Revert` | 從保留的位元組重新開啟，再做一次編輯 | `engine.rs` | worker `saving.rs`、`merge.rs` | 沒有（#427） |
| `Rebase` | 把剛存的檔案當成 undo 的起點；只保留位元組，不解析 | `serve.rs` | worker `saving.rs` | 不需要 |
| `Save` | 寫出整份文件；已簽章的文件以增量更新 | `engine.rs` | worker `saving.rs`、`save_poc.rs` | 只有文字方塊那條路（`text_box_font`）；#427 |
| `SavePages` | 只寫出某些頁 | `engine.rs` | worker `saving.rs`；`split.spec.ts` | 沒有（#427） |
| `PrivacyCopy` | 不含中繼資料的副本 | `privacy.rs` | `privacy-export.spec.ts` | 沒有（#427） |
| `CompressedCopy` | 解碼圖片再重新編碼 | `compress.rs`、`jpeg.rs` | worker `compress.rs` | `compress_images` |
| `ExifCleanCopy` | 拿掉圖片裡的中繼資料，不重新編碼 | `exif.rs` | worker `exif_clean.rs` | `exif_clean` |
| `SanitizedCopy` | 清理過的副本 | `engine/sanitize.rs` | worker `sanitize.rs` | 沒有（#427） |
| `EncryptedCopy` | 加密的副本；密碼用完即清除 | `engine.rs` | worker `encrypt_copy.rs` | 沒有（#427） |
| `WatermarkCopy` | 加上文字或圖片浮水印的副本 | `engine/watermark.rs` | worker `watermark.rs` | 沒有（#427） |
| `PageMarksCopy` | 加上頁面標記的副本 | `engine/page_marks.rs` | worker `page_marks.rs` | 沒有（#427） |
| `WriteIdentifier` | 以增量更新寫入或拿掉 app 的文件識別碼（ADR 0010） | `engine/identifier.rs` | worker `sensitive.rs` | 沒有（#427） |
| `PrepareSignature` | 以增量更新加上簽章欄位，留空位給 CMS | `engine/signing.rs` | `documents.rs`（`a_prepared_copy_is_signed_in_its_room_and_nowhere_else`）；`signing.spec.ts` | 沒有（#427） |
| `PrepareSource` | 另一個 PDF（唯讀代號、可能的密碼）做成乾淨的副本 | `engine.rs` | worker `merge.rs` | 沒有（#427） |
| `PrepareStampImage` | PNG 或 JPEG（≤ 16 MiB） | `engine/stamp_image.rs` | worker `stamp_image.rs`、`watermark.rs` | `stamp_image`（#428） |
| `PrepareTextBox`、`PrepareTextBoxes` | 字型檔（≤ 64 MiB） | `engine/text_box.rs`、`engine/text_box/` | worker `text_box_worker.rs`、`fonts.rs` | `text_box_font` |
| `InspectFont` | 要匯入的字型檔 | `engine/text_box/font.rs` | worker `text_box_worker.rs` | `text_box_font`（#428） |
| `CreatePdf` | 圖片、HTML、純文字、EPUB、XPS、SVG、docx、xlsx、pptx（唯讀代號） | `engine/create.rs` | worker `create_pdf.rs` | `create_pdf` |
| `OcrLoad` | `.traineddata`（≤ 64 MiB；worker 再跑一次 `check_language_data`） | `ocr.rs` | worker `ocr.rs` | 沒有（#431） |
| `OcrPage`、`OcrPoll`、`OcrStop` | 掃描頁：判斷、渲染成灰階、辨識、做成文字層 | `engine/ocr.rs`、`ocr_worker.rs`、`text_layer.rs` | worker `ocr.rs`、`search_cache.rs` | 沒有（#431） |
| `Close` | 關閉文件 | `serve.rs` | `tabs.spec.ts` | `worker_messages` |
| `Cancel` | 主行程目前不送；worker 收到也不處理 | `serve.rs` | — | `worker_messages` |
| `Shutdown` | 主行程目前不送（直接結束 worker）；worker 收到就結束 | `serve.rs` | `crates/ipc_contract/src/frame.rs` | `worker_messages` |

### 2.5 編輯（`WorkerEdit`，29 種）

前端送的是 `Edit`，主行程檢查後轉成 `WorkerEdit`。插入的頁面來源與文字方塊的字型子集是主行程留著的，前端不能直接給位元組。

| 編輯 | 主行程的檢查（`Edit::validate`） |
|---|---|
| `RotatePages`、`DeletePages`、`MovePages` | 頁面清單（`check_page_list`）；位置 ≤ `MAX_PAGE_COUNT` |
| `InsertBlankPage`、`InsertPages` | 位置 ≤ `MAX_PAGE_COUNT`；`InsertPages` 的來源是 `PrepareSource` 做好的副本 |
| `AddHighlight`、`SetHighlightColor` | 頁碼（`check_annotated_page`）；頁數 ≤ `MAX_HIGHLIGHT_PAGES`；四邊形不得為空、總數 ≤ `MAX_ANNOTATION_QUADS`；顏色是固定的選項 |
| `AddNote`、`SetNoteText` | 頁碼；位置；文字過 `check_note_text` |
| `DeleteAnnotation`、`SetAnnotationRect`、`SetPictureAngle` | 頁碼；矩形（`check_annotation_rect`）；角度（`check_angle`） |
| `SetFieldValue`、`FlattenForm` | 頁碼；值過 `check_field_value` |
| `AddRedaction`、`AddRedactionGroup`、`ApplyRedactions` | 頁碼；矩形；一組 ≤ `MAX_REDACTION_GROUP_RECTS` |
| `AddInk` | 頁碼；筆畫不得為空，≤ `MAX_INK_STROKES`；點數 ≤ `MAX_INK_POINTS`；顏色與筆寬是固定的選項 |
| `AddShape`、`SetLineEnds`、`SetShapeStyle` | 頁碼；矩形或線段端點座標；形狀、箭頭、顏色與線寬是固定的選項 |
| `AddStamp`、`AddImageStamp` | 頁碼；矩形；圖章名稱是固定的選項；圖片是 `PrepareStampImage` 做好的 PNG |
| `SetTextBox`、`PlaceTextBox` | 頁碼；矩形；樣式（`TextBoxStyle::validate`）；字是 `PrepareTextBox` 做好的模型 |
| `AddBookmark`、`RenameBookmark`、`DeleteBookmark`、`MoveBookmark` | 位置 ≤ `MAX_OUTLINE_ITEMS`；頁碼；標題不得空白、≤ `MAX_TEXT_BYTES`、不含控制字元（`check_clean`） |

測試：`documents.rs` 的編輯測試（`annotations_are_added_changed_removed_and_undone_like_any_edit`、`drawings_and_stamps_are_added_moved_removed_and_undone_like_any_annotation` 等）、`ipc_contract` 的驗證測試、worker 的整合測試與上面列的 E2E。fuzz：沒有（#427）。

## 3. 主行程自己解析的資料

主行程沒有沙盒，這裡的程式碼出錯（panic、一次配置太多記憶體）時整個 app 會結束。

| 資料 | 從哪裡來 | 怎麼檢查 | 測試 | fuzz |
|---|---|---|---|---|
| `.pfx`、`.p12` | 使用者選的檔案（≤ 1 MiB） | Windows 的 `PFXImportCertStore`，加 `PKCS12_NO_PERSIST_KEY \| PKCS12_ALWAYS_CNG_KSP`：金鑰不存到磁碟、不能匯出。憑證鏈加 `CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL`、`CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE`，不連網（ADR 0020） | `signing.rs` 11 個（`the_test_certificate_signs_two_parts_and_leaves_no_key_behind`、`a_wrong_password_or_a_file_that_is_no_certificate_says_only_that` 等） | 不做：Windows 的程式碼，ADR 0020 接受的設計（§9） |
| `.traineddata` | 使用者匯入（≤ 64 MiB） | `check_language_data`：目錄的項目數 23–64，位移不倒退、不超出檔案，要有 LSTM 模型、字元集與 recoder | `crates/ipc_contract/src/ocr.rs` 8 個 | `language_data`（#429） |
| 字型 | 使用者匯入（≤ 64 MiB） | 副檔名、大小、`check_import_header`；之後由新的 worker 檢查（`InspectFont`） | `fonts.rs` 15 個（含檔頭與中繼資料的截斷／變異測試） | 檔頭由單元變異測試驗證（#429）；`InspectFont` 有（`text_box_font`，#428） |
| 檢查更新的回覆 | 網路（§5） | ≤ 1 MiB；serde_json；只讀版本號 | `update_check.rs` 7 個（含 `interpret` 的截斷／變異測試） | 由單元變異測試驗證（#429） |
| `settings.json`、`recent.json`、`sensitive.json`、`fonts\metadata.json` | app 資料夾 | 先限制大小（`local_data.rs`：`settings.json` 64 KiB、`recent.json` 512 KiB、`sensitive.json` 2 MiB、`metadata.json` 512 KiB），不是 app 寫的內容就當成沒有 | `local_data.rs`、`settings.rs`、`recent.rs`、`sensitive.rs`、`fonts.rs`（各模組皆有截斷、變異與隨機輸入測試） | 由單元變異測試驗證（#429） |
| 復原日誌 | app 資料夾的 `recovery\`（每個 ≤ 4 MiB） | 當成不可信的資料檢查 | `recovery.rs` 15 個（含截斷、變異與隨機輸入測試） | 由單元變異測試驗證（#429） |
| 文字索引 | app 資料夾的 `index\`（金鑰 ≤ 4 KiB，目錄與文件 ≤ 32 MiB） | AES-256-GCM；金鑰以 DPAPI 保護。不是 app 寫的檔案在解析前就被拒絕 | `text_index.rs` 24 個（`nothing_on_the_disk_can_be_read_without_the_key` 等）、`index_crypto.rs` 3 個 | 不需要（先驗證才解析） |
| 認證管理員裡的密碼 | app 自己寫的項目（`PdfReader/password/v1/<key>`） | 鍵是 worker 從檔案的加密資訊算出的雜湊（`PasswordKey`，ADR 0017）；項目裡沒有檔名、路徑或內容 | `credentials.rs` 7 個 | 不需要 |
| 命令列、第二次啟動的參數 | 其他程式（§8） | 只當成檔案路徑 | `cli.rs` 3 個 | 不需要 |

## 4. WebView 的權限

| 項目 | 設定 | 檢查 |
|---|---|---|
| capability | `src-tauri/capabilities/main.json`：只給 `main` 視窗；89 個 `allow-*`；沒有 `core:*`、`dialog`、`fs`；沒有 `remote`，外部網頁拿不到 IPC | `check-security-config.mjs`（靜態）；`refused-commands.spec.ts`（#425） |
| CSP | `default-src 'self'`、`script-src 'self'`、`style-src 'self'`、`img-src 'self' data: blob:`、`font-src 'self'`、`connect-src ipc: http://ipc.localhost`、`object-src`／`base-uri`／`form-action`／`frame-src` 都是 `'none'` | `check-security-config.mjs`（正式版與 `vite.config.ts` 的開發用 CSP）；`check-dist.mjs`（建置產物不引用外部資源） |
| 全域 API | `withGlobalTauri: false` | — |
| 開發人員工具 | `tauri` 沒有開 `devtools` 功能，正式版沒有開發人員工具 | 人工檢查（manual-checks §3） |
| WebView2 的參數 | `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-background-networking --no-proxy-server --disable-component-update` | `network.spec.ts`（#121） |
| 導覽 | 主視窗設定 `on_navigation`（`src-tauri/src/navigation.rs`）：只允許 app 自己的頁面（正式版 `http://tauri.localhost`，debug `http://localhost:1420`），其餘 scheme、外部網域、帶認證或不同 port 一律拒絕 | `navigation.rs` 7 個；`navigation.spec.ts`（#424） |
| 下載 | 主視窗設定 `on_download`，一律回傳 `false`（WebView2 `DownloadStarting` 取消）：頁面無法透過 `<a download>`、`blob:`、`data:` 下載任何檔案 | `navigation.spec.ts`（#424） |
| 新視窗 | 沒有處理函式時，wry 0.57 把 `NewWindowRequested` 設成已處理，不開視窗 | — |
| 剪貼簿 | 頁面可以寫，不能讀 | `clipboard.spec.ts`（#209） |
| 列印 | 頁面的 `window.print` 打開 WebView2 的列印對話框 | `print.spec.ts` |
| 前端程式碼 | 沒有 `innerHTML`、`eval` 與 `<a href>`；ESLint 禁用網路 API；只有 `print/render.ts` 用 `createObjectURL`；`navigator.clipboard.writeText` 只在 `LinkDialogs.tsx` 與 `ReaderShell.tsx` | `pnpm lint` |

## 5. 網路

只有一個地方會連網（ADR 0009，[update-check.md](../architecture/update-check.md)）：

- **在哪裡**：`src-tauri/src/update_check.rs`，主行程，只在使用者按「檢查更新」（`check_for_updates`）時。
- **怎麼連**：WinHTTP，TLS 1.2 或 1.3，固定的 GitHub 網址；不帶 cookie，不做驗證，不跟隨轉址，不保持連線；用 WinHTTP 預設的 proxy 設定；逾時 15 秒。
- **讀什麼**：回覆 ≤ 1 MiB，只讀版本號；同時只跑一個。
- **測試**：`only_the_fixed_address_is_ever_asked`、`one_check_at_a_time` 等 6 個。

其他地方都不連網：

| 位置 | 怎麼做到 | 檢查 |
|---|---|---|
| worker | AppContainer 沒有網路的 capability；執行檔不匯入網路 API | `network_is_blocked_even_to_localhost`、`worker_binary_imports_no_networking` |
| WebView | CSP；WebView2 的參數；`on_navigation` 只允許 app 自己的網址，`on_download` 封鎖下載 | `network.spec.ts`（WebView 什麼都不連）、`remote-references.spec.ts`（遠端參照一律不載入，ADR 0022）、`navigation.spec.ts`（#424） |
| 依賴與程式碼 | 不得有 HTTP client、更新器、遙測 | `scripts/ci/check-forbidden.sh`、`scripts/ci/forbidden-patterns.txt`、`cargo deny check` |
| 整個 app | 實際執行時觀察 | [offline-verification.md](offline-verification.md)；manual-checks §1 |

## 6. 會寫入的位置

| 位置 | 寫什麼 | 何時 |
|---|---|---|
| app 資料夾 `%LOCALAPPDATA%\io.github.winner0988.pdfreader\`（或 `PDF_READER_DATA_DIR`，§8） | `recent.json`、`settings.json`、`sensitive.json`、`tessdata\`（匯入的語言）、`fonts\` 與 `metadata.json`（匯入的字型）、`recovery\`（復原日誌）、`index\`（文字索引）、`EBWebView\`（WebView2 自己的資料） | 使用時；細節見 [local-data.md](../architecture/local-data.md) |
| AppContainer 的設定檔 `%LOCALAPPDATA%\Packages\pdfreader.worker\`，與 HKCU 裡的對應 | Windows 建立 AppContainer 時寫入 | 第一次啟動 worker；解除安裝時刪除 |
| 使用者選的位置，與旁邊的暫存檔 | 另存、匯出、副本、附件、簽章的副本、建立的 PDF | 使用者在對話框選好之後 |
| 文件自己的檔案 | 存檔、機敏標記 | 使用者存檔或確認標記之後 |
| 批次選的資料夾 | 副本（不重複的新檔名）、報告（`create_new`） | 批次執行時 |
| 附件的 `Zone.Identifier` 資料流 | 標成來自網際網路 | 存附件時 |
| 認證管理員 | `PdfReader/password/v1/<key>` | 使用者要記住密碼時 |
| 剪貼簿 | 複製的文字 | 使用者複製時 |
| worker 執行檔的 ACL | 讓 AppContainer 能執行 | 執行檔在使用者擁有的位置時（例如開發建置）；安裝在 Program Files 時不會發生 |
| `Program Files`、HKLM 的檔案關聯 | 安裝程式（NSIS，per-machine） | 安裝、解除安裝 |

產品程式碼不寫 `%TEMP%`。

## 7. 會啟動的行程

| 行程 | 由誰啟動 | 檢查 |
|---|---|---|
| `pdf_worker.exe` | `crates/sandbox` 的 `Sandboxed::spawn`（`CreateProcessAsUserW`），每個分頁一個，另有 §2.4 的幾種一次性的 worker | §2.1 |
| 預設的瀏覽器或郵件程式 | `src-tauri/src/opener.rs` 的 `ShellExecuteW("open")`，只由 5 個命令觸發：`open_link`、`open_outline_link`、`open_remote_address`（都經過 `links::preview`）、`open_releases_page`（常數）、`open_default_apps_settings`（常數 `ms-settings:`）。網址是 `ShellExecuteW` 唯一的檔案參數，不組命令列 | `links.rs`；`opener.rs` 沒有測試；manual-checks §2 |
| WebView2 的行程 | WebView2 自己 | — |

`taskkill` 只在測試中使用。WebView2 不在時，app 以 `rfd` 顯示訊息方塊，不啟動其他程式。

## 8. 其他程式能影響 app 的方式

| 入口 | 處理方式 | 檢查 |
|---|---|---|
| 命令列 | `src-tauri/src/cli.rs`：跳過 `-` 開頭的參數，其餘當成檔案路徑交給 `commands::open_paths`（開啟對話框也走這裡） | `cli.rs` 3 個；`open.spec.ts` |
| 第二次啟動 | `tauri-plugin-single-instance` =2.5.4：mutex、`FindWindowW`、`WM_COPYDATA` 傳來參數與工作目錄，主行程以 `Path::new(&cwd).join(path)` 交給 `open_paths` | `batch.spec.ts`（第二次啟動把檔案交給視窗） |
| 拖放 | 視窗的拖放事件把每個拖進來的檔案交給 `open_paths`（每個檔案各開一個分頁；非 PDF 或資料夾則顯示錯誤分頁） | manual-checks §3.14（原生 OLE 拖放無法由 Playwright CDP 模擬，見 `docs/architecture/e2e.md`） |
| 檔案關聯 | 安裝程式在 HKLM 登錄 `.pdf` 的開啟命令 | manual-checks §3 |
| `PDF_READER_DATA_DIR` | 是絕對路徑時，取代 app 資料夾；正式版也讀（E2E 用它讓每次執行有自己的資料夾） | §9 |
| WebView2 的環境變數 | `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`、`WEBVIEW2_USER_DATA_FOLDER` 等由 WebView2 自己讀；E2E 用前者打開 `--remote-debugging-port`（`docs/architecture/e2e.md`） | §9；manual-checks §3 |
| 只給測試用的環境變數 | `CI`、`PDF_READER_TEST_CLIPBOARD`、`PDF_READER_TEST_SIGNING` 只在單元測試裡讀 | — |

## 9. 只在畫面上的確認、剩餘風險與已接受的設計

這些不開 Issue：有的是 ADR 已接受的設計，有的是只能記錄、由人工檢查確認的風險。

- **只在 WebView 裡做的確認**：開啟連結前的確認、危險附件的警告、套用遮蔽前的摘要、未存的變更。頁面被注入程式碼時可以略過這些確認，但主行程仍然限制結果：只開 `http`、`https`、`mailto` 的連結，附件只存不開，遮蔽與存檔仍然由使用者的檔案選擇決定。
- **IPC 的大小**：主行程不限制一個 IPC 訊息的大小，參數整個解碼後才檢查。被注入的頁面可以送很大的參數讓主行程用掉記憶體；這和它本來就能做的事（例如一直呼叫命令）相比，不會多拿到什麼。
- **沒有用 Tauri 的 Isolation pattern**：靠 CSP、capability 與「前端不碰路徑」。
- **同一個使用者的其他程式**：能用 `WM_COPYDATA`、命令列、`recent.json` 讓 app 開某個檔案，能設環境變數（`PDF_READER_DATA_DIR`、WebView2 的變數，例如打開除錯埠）。這些程式本來就能以同樣的權限做任何事，所以不當成邊界；但 1.0 前要人工確認正常啟動時沒有除錯埠（manual-checks §3）。
- **`recent.json` 等設定檔沒有驗證**：只檢查大小與格式。同上，改得到它的程式已經有使用者的權限。
- **PFX 由 Windows 在主行程解析**（ADR 0020，選項 B）：簽章用的私密金鑰要留在主行程，不交給 worker。
- **不是 LPAC**；執行檔在使用者擁有的位置時改 ACL（§2.1）。
- **`pick_stamp_image` 先開對話框才檢查文件**：文件不能加註解時，使用者選完圖片才看到錯誤。不影響安全。
- **`Cancel` 與 `Shutdown` 主行程不送**：主行程以結束 worker（Job Object）代替。

## 10. 缺口與 Issue

| 缺口 | Issue | 需要安全審查 |
|---|---|---|
| 已在 #422 修正：請求期限從送出起算、不相符編號與種類不對時終止 worker 並標記 lost，以 fake_worker 測試協定違規（§2.2） | #422 | 是 |
| 匯出純文字能取代文件自己的檔案（§1.8，已於 #423 修復） | #423 | 是（已修復） |
| WebView 的導覽沒有限制（§4，已於 #424 加上 on_navigation 與 on_download） | #424 | 是（已修復） |
| 執行中的 app 會拒絕沒授權的命令（§4） | 已由 #425 補上測試（`refused-commands.spec.ts`） | 否 |
| 已由 #426 拆出 `read_document` 目標完成 fuzz（§2.4） | #426 | 是 |
| fuzz 沒有走到的編輯、存檔、副本（§2.4、§2.5） | #427 | 是 |
| 已在 #428 補上：圖章圖片（`PrepareStampImage`，新目標 `stamp_image`）與匯入字型（`InspectFont`，併入 `text_box_font`）的 fuzz（§2.4） | #428 | 是 |
| 主行程自己解析的資料沒有 fuzz（§3；已由 #429 補上 language_data fuzz 目標與主行程解析器變異測試） | #429 | 是 |
| OCR 的語言資料與掃描頁沒有 fuzz（§2.3、§2.4） | #431 | 是 |
| `retry_open`、`cancel` 在主行程這一側沒有測試（§1.1、§1.2；已由 #432 於 `retry-open-and-cancel.spec.ts` 測試） | #432 | 否 |
| 拖放開檔列入人工檢查（manual-checks §3.14），`commands.rs` 註解已更正（§8） | #436（已完成） | 否 |
| 這份清單與程式碼一致的 CI 檢查 | 已由 #430 實作（`check-security-config.mjs`） | 是 |
