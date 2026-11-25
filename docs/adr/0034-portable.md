# ADR 0034：可攜版（免安裝）：另一個建置，解開的資料夾就能用，裡面附 Fixed Version 的 WebView2；資料都在程式旁邊的 `data\`；支援 64 位元的 Windows 10 22H2 與 Windows 11；正常結束時刪除 worker 的 AppContainer profile

## 狀態
已接受

（負責人於 2026-11-24 在 Claude Code 的對話中接受〔[#449 的留言](https://github.com/winner0988/Pdf-reader/issues/449#issuecomment-7146640688)〕：第 1、2、3 組選建議的做法；第 4 組改選 C（資料夾裡附 Fixed Version 的 WebView2，檔案變大沒關係）；第 5 組改選 B（也在 exFAT、FAT32 上執行，實作前先做 POC）；並要求可攜版在 Windows 11 以外也能用，所以加了第 6 組。工作卡 B5-12，[#449](https://github.com/winner0988/Pdf-reader/issues/449)，排在 1.2.0〔追蹤卡 [#690](https://github.com/winner0988/Pdf-reader/issues/690)〕。來源是 2026-11-03 與 pdfcraft 的比較；規格 §1；[local-data.md](../architecture/local-data.md)。粗體是負責人選定的做法；實作前先做下面的 POC，不通過就不實作。）

## 背景
- **安裝版**（[packaging.md](../architecture/packaging.md)）：
  - NSIS 安裝檔，`perMachine`：裝在 `C:\Program Files\PDF Reader\`，安裝時要 UAC。只有系統管理員能寫入那裡，所以一般權限的程式換不掉 `pdf_worker.exe`；Program Files 允許所有 app package 讀取與執行，AppContainer 直接啟動 worker。
  - 安裝時註冊「開啟檔案」與預設 PDF 閱讀器的設定（規格 §1）。
  - 不簽章（DEC-04），發行時附 SHA-256，SmartScreen 會警告「不明的發行者」。
  - 不下載 WebView2：只支援內建 WebView2 的 Windows 11（REL-02）；缺少時 app 說明並結束。
- **app 的資料**（local-data.md）全部在 `%LOCALAPPDATA%\io.github.winner0988.pdfreader\`：設定、最近開啟的檔案（**完整路徑**）、機敏文件的清單（識別碼與檔名）、匯入的語言資料與字型、崩潰復原的日誌（**完整路徑**與還沒存檔的編輯）、本機全文索引（預設關閉）、簽名圖片、WebView2 的資料（`EBWebView\`）。只有主行程讀寫，讀的時候都當成不受信任的資料。`PDF_READER_DATA_DIR`（絕對路徑）可以換位置，E2E 用它隔離每次執行。
- **不在那個資料夾、但留在這台電腦上的**：
  - worker 的 AppContainer profile：`%LOCALAPPDATA%\Packages\pdfreader.worker\`（容器的 `TEMP`、`TMP`，不含使用者資料）與 HKCU 的對應；解除安裝時刪除（`installer-hooks.nsh`），程式內有 `sandbox::delete_app_container_profile`；
  - 記住的密碼：Windows 認證管理員（ADR 0017）；
  - 以 DPAPI 保護的全文索引金鑰與簽名圖片：只有這台電腦上的這位 Windows 使用者解得開。
- **AppContainer 與檔案的權限**（[worker-sandbox.md](../architecture/worker-sandbox.md)）：容器只能開啟權限（ACL）明確授權給它的檔案。worker 執行檔在使用者擁有的位置時，啟動前把那個執行檔的讀取＋執行權限授予容器。改 ACL 要 `WRITE_DAC`（檔案的擁有者有）；改不了時（`ERROR_ACCESS_DENIED`），`sandbox` 的 `grant_read_execute` 當成成功，因為安裝版在 Program Files，本來就授權給所有 app package。
- **單一執行個體**（ADR 0012）：第二次啟動把檔案交給已經開著的視窗再結束，以 Tauri 設定的 `identifier`（`io.github.winner0988.pdfreader`）區分，它在建置時就決定了。
- **更新**（ADR 0009，[update-check.md](../architecture/update-check.md)）：只有使用者按下「檢查更新」時，主行程送一個請求到固定的 GitHub API；有新版時，確認後以系統瀏覽器開啟固定的發行頁。不自動下載、不安裝。
- **WebView2 的 Fixed Version**（微軟的[散布說明](https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution)，2026-10-02 的版本）：
  - 下載指定的版本，和 app 放在一起散布；超過 250 MB。
  - 不會自己更新：要拿到安全修正，app 就要附上新的版本重新發行。微軟只提供最新兩個大版本的下載。
  - 不能從網路上的位置或 UNC 路徑執行（已知問題）。
  - Windows 10 上，從 WebView2 120 版起，未封裝的 Win32 app 用 Fixed Version 時，要把那個資料夾的讀取＋執行權限授予 ALL APPLICATION PACKAGES（`S-1-15-2-1`）與 ALL RESTRICTED APPLICATION PACKAGES（`S-1-15-2-2`），因為 renderer 改在 App Container 中執行；Windows 11 不用。
  - Tauri 2.12.4：`bundle.windows.webviewInstallMode` 是 `fixedRuntime` 時，在建立 runtime 之前把 `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` 設成「程式所在的資料夾＋設定的 `path`」（`tauri` 的 `app.rs`），WebView2 就只用那一份。
- **Windows 10**：
  - 2025-10-14 結束一般的支援；之後只有加入延長安全更新（ESU）的電腦還有安全更新。家用版的 ESU 延長到 2027 年 10 月（微軟 2026-06 宣布）。
  - 微軟在 Windows 10 22H2 上更新 Edge 與 WebView2 到至少 2028 年 10 月，不需要 ESU。
  - Rust 1.78 起，`x86_64-pc-windows-msvc` 的最低版本是 Windows 10（Windows Server 2016）；Windows 7、8.1 只有另一個第三級的目標，不能用。
  - app 本身：主程式與 worker 的 C、C++ 執行階段都是靜態連結（packaging.md），只用 Windows 10 起內建的 UCRT；worker 的沙盒用 AppContainer、Job Object，以及 Windows 10 就有的緩解措施；防截圖的 `WDA_EXCLUDEFROMCAPTURE` 要 Windows 10 2004 起（[sensitive-files.md](../architecture/sensitive-files.md)）。CI 只在 `windows-latest`（Windows Server 2025）上跑，沒有在 Windows 10 上跑過。
- **原則**：不連網、零遙測（AGENTS.md 原則 1、2）；worker 的隔離（原則 4）；最小權限（原則 6）。

## 選項

### 1. 怎麼知道是可攜版

| 選項 | 換來 | 付出與風險 |
|---|---|---|
| **A. 另一個建置（負責人選定）** | 是不是可攜版在建置時就決定，判斷時不看任何檔案、參數或環境變數：別人在安裝版旁邊放一個檔案，什麼都不會改變。`identifier` 也不同，單一執行個體與安裝版分開 | CI 與發行多建置一次主程式；可攜版要另外測（工作卡已經要求 E2E 跑一次） |
| B. 程式旁邊的標記檔（工作卡的例子） | 只有一個建置 | 安裝版也要在啟動時找標記檔：能在程式的資料夾放檔案的人就能切換模式（安裝版的資料夾只有系統管理員能寫，風險不大，但多了一個輸入）。`identifier` 是建置時決定的，單一執行個體分不開：安裝版開著時，從隨身碟開的檔案會交給安裝版，記進這台電腦的最近開啟的檔案 |
| C. 命令列參數（`--portable`） | 只有一個建置 | 直接按兩下執行檔就不是可攜版；捷徑在每台電腦上都要重做；單一執行個體同樣分不開 |

### 2. 每一項資料在可攜版怎麼做

| 項目 | 安裝版 | 可攜版（負責人選定） | 另一種做法 | 為什麼 |
|---|---|---|---|---|
| 最近開啟的檔案 | 預設開啟 | **預設關閉**，可以在設定中開啟（開的時候說明風險） | 與安裝版相同 | 完整路徑含 Windows 的使用者名稱與檔名，隨身碟遺失就外流；換電腦後磁碟代號不同，路徑多半也不對 |
| 崩潰復原的日誌 | 開啟 | **開啟**，在 `data\` | 關閉 | 只在有未儲存的變更、app 又沒有正常結束時才留下；關掉的話，當機就丟了編輯 |
| 本機全文索引 | 預設關閉 | **不提供** | 每台電腦各自一份 | 金鑰以 DPAPI 保護，換一台電腦（或換一位 Windows 使用者）就解不開，留在隨身碟上也沒用 |
| 簽名圖片 | 有 | **不提供**，每次從檔案選圖片 | 不加密存在 `data\` | 以 DPAPI 加密，換電腦就解不開；不加密放在隨身碟上，拿到的人就能用你的簽名 |
| 記住密碼 | 有 | **不提供** | 與安裝版相同 | 密碼存在那台電腦的 Windows 認證管理員，可攜版就在那台電腦上留下了東西 |
| 設為預設 PDF 閱讀器 | 有 | **不提供**（工作卡的「不做」） | — | 要寫登錄檔 |
| 設定、機敏文件的清單、匯入的字型與語言資料 | 有 | **照常**，在 `data\` | — | 沒有路徑；機敏文件的清單只有識別碼與檔名 |

### 3. worker 的 AppContainer profile

| 選項 | 換來 | 付出與風險 |
|---|---|---|
| **A. 正常結束時刪除（負責人選定）** | 用過的電腦不留下 profile，不必記得去清 | 每次啟動都要重新建立（POC 量時間）；當機時留下的，要等下次在這台電腦上正常結束才刪掉 |
| B. 只提供「清除這台電腦上的資料」 | 啟動時不用重新建立 | 使用者要記得在每台電腦上按；不按就留著 |
| C. 不處理 | 最簡單 | 每台用過的電腦都留著一個 profile（`%LOCALAPPDATA%\Packages\` 與 HKCU），沒有解除安裝程式會刪它，達不到工作卡的驗收 |

### 4. WebView2

| 選項 | 換來 | 付出與風險 |
|---|---|---|
| A. 系統的 WebView2；資料資料夾在 `data\EBWebView\`，以 InPrivate 開啟 | 資料夾小；WebView2 由 Windows 更新 | 沒有 WebView2 的電腦（多半是 Windows 10）不能用 |
| B. 系統的 WebView2；資料資料夾在 `%TEMP%`，結束時刪除 | 啟動快 | 畫面元件的行程結束之後才刪得掉；當機時留在這台電腦上，要等下次啟動再清；沒有 WebView2 的電腦一樣不能用 |
| **C. 資料夾裡附 Fixed Version 的 WebView2；資料資料夾在 `data\EBWebView\`，以 InPrivate 開啟（負責人選定）** | 沒有 WebView2、不能連網的電腦也能用；每台電腦用的是同一個版本 | 資料夾大 250 MB 以上；WebView2 不會自己更新，安全修正要靠我們重新發行；Windows 10 上要先授權（見「WebView2」） |

### 5. 檔案系統

| 選項 | 換來 | 付出與風險 |
|---|---|---|
| A. 只在 NTFS 或 ReFS 的磁碟上執行 | worker 的隔離靠 AppContainer，AppContainer 靠檔案的權限決定它能碰什麼；NTFS 與 ReFS 才有檔案的權限 | 新的隨身碟多半是 exFAT 或 FAT32，要先格式化成 NTFS；啟動時檢查，不是就說明並結束 |
| **B. 也在 exFAT 與 FAT32 上執行（負責人選定；實作前先做 POC）** | 隨身碟不用格式化 | 這兩種沒有檔案的權限，程式與 `data\` 所在的磁碟可能少了這層保護：worker 被攻破時，可能讀寫整個隨身碟，連程式本身都換得掉。POC 先量清楚；隔離比較弱時，由負責人選怎麼做（見「檔案系統」） |

### 6. 可攜版支援哪些 Windows

| 選項 | 換來 | 付出與風險 |
|---|---|---|
| A. 只支援 Windows 11，與安裝版相同 | 只測一種系統 | 還沒升級的 Windows 10 電腦不能用 |
| **B. 64 位元的 Windows 10 22H2 與 Windows 11（負責人選定）** | 還沒升級的 Windows 10 電腦也能用 | 要多測一種系統；Windows 10 上 WebView2 要另外授權；沒有加入 ESU 的 Windows 10 已經沒有安全更新，系統的漏洞 app 補不了 |
| C. 再加上 Windows 7、8.1 | — | 做不到：Rust 1.78 起，標準的 Windows 目標最低是 Windows 10；WebView2 也早就不再支援這兩個版本 |

## 決定（負責人選定：1 的 A、2 的「可攜版」那一欄、3 的 A、4 的 C、5 的 B、6 的 B）

### 範圍：第一版
- 一個 zip，裡面一個資料夾：主程式、`pdf_worker.exe`、`tessdata\`（內附的 `eng`、`chi_tra`）、`WebView2\`（Fixed Version）與授權檔；另附 `.sha256`。檔名由實作卡訂。
- 解開後執行主程式就能用，不用安裝，也不用連網；第一次執行時在程式旁邊建立 `data\`。
- 支援 64 位元的 Windows 10 22H2 與 Windows 11（選項 6 的 B）。
- 不做（工作卡的「不做」）：自動更新；設為預設 PDF 閱讀器。

### 可攜版是另一個建置
- Tauri 設定的 `identifier` 不同（例如 `io.github.winner0988.pdfreader.portable`），以 `tauri build --config` 加上另一個設定檔（與 `tauri.bundle.conf.json` 相同的做法）。程式以建置時的設定知道自己是可攜版，執行時不看任何標記檔、參數或環境變數。
  - 所以在安裝版旁邊放什麼檔案，安裝版都不會改變；
  - 單一執行個體以 `identifier` 區分：可攜版與安裝版同時開著時，各自開自己的檔案，不會交給對方。
- worker 的 AppContainer 用另一個名稱（例如 `pdfreader.portable.worker`），刪除它不影響安裝版。`pdf_worker.exe` 是同一個，worker 不知道自己在哪一版。
- **設定檔與檢查**：`check-security-config.mjs`（#693）在 `src-tauri/` 只接受 `tauri.conf.json` 與 `tauri.bundle.conf.json`（`TAURI_CONFIG_FILES`）；bundle 的設定檔也只能設 `$schema`、`build.beforeBuildCommand`、`bundle.externalBin` 三個鍵（`BUNDLE_CONFIG_SETTINGS`）。可攜版的設定檔放在 `src-tauri/` 會被它擋下；放在別處，`tauri build --config` 讀得到、檢查卻讀不到，那個檔就能放寬 CSP 或 capability，CI 看不出來。所以：
  - 可攜版的設定檔放在 `src-tauri/`（例如 `tauri.portable.conf.json`），加進檢查的清單，只允許設 `identifier`、`bundle.windows.webviewInstallMode`（只能是 `type` 為 `fixedRuntime`、`path` 為 `WebView2`）與 bundle 設定檔的那三個鍵，其他的鍵都讓檢查失敗；
  - 可攜版其他的不同（資料在 `data\`、InPrivate 與畫面元件的資料資料夾、AppContainer 的名稱、不提供的項目、Windows 10 上的授權）不放在設定檔，由主程式在編譯時決定（例如 Cargo 的 feature `portable`）；
  - 改這個設定檔、改檢查，都要 `needs-security-review`。實作的 PR 會改 `scripts/ci/`，要負責人核准。
- **環境變數**：上面的「判斷時不看」指的是判斷是不是可攜版。可攜版**不讀 `PDF_READER_DATA_DIR`**：資料一律在程式旁邊的 `data\`，不會因為這台電腦設了這個變數就寫到別處。E2E 每次把可攜版解壓縮到新的暫存資料夾，`data\` 就在那裡，不用這個變數也能隔離每次執行。其他不改變資料位置的變數（例如測試用的 `PDF_READER_TEST_WINDOWS_MODE_KEY`）與安裝版相同。

### 資料放在哪裡
- 全部在程式旁邊的 `data\`：內容與規則與安裝版的資料夾相同（local-data.md），讀的時候照樣當成不受信任的資料。
- 每一項的預設照選項 2 選定的那一欄。可攜版不提供的項目，主行程也拒絕，不只是畫面上不顯示。
- 程式的資料夾不能寫入時（唯讀的磁碟、`Program Files`），啟動時說明並結束：可攜版要能在程式旁邊建立 `data\`。
- 資料夾的權限沿用所在的位置，app 不另外設定：放在自己的使用者資料夾時，只有自己（與系統管理員）讀得到；放在隨身碟時，拿到隨身碟的人都讀得到。

### 支援的 Windows（選項 6 的 B）
- 64 位元的 Windows 10 22H2 與 Windows 11。安裝版照舊只支援 Windows 11（REL-02）。
- Windows 10 的其他版本不測、不保證，啟動時也不擋；worker 或畫面開不起來時，照現在的錯誤處理說明。
- 和 Windows 11 不同的地方：
  - WebView2 要另外授權（見「WebView2」）；
  - 防截圖要 Windows 10 2004 起，22H2 有；
  - 「檢查更新」在 Windows 10 只用 TLS 1.2（update-check.md，現在就是這樣）。
- Windows 10 本身的安全更新不在 app 的控制內：沒有加入 ESU 的 Windows 10 已經沒有安全更新，沙盒依靠的系統元件有漏洞時，app 補不了。使用手冊寫明。
- 微軟停止在 Windows 10 上更新 WebView2 之後（目前說至少到 2028 年 10 月），可攜版要不要繼續支援 Windows 10，由負責人再決定。

### WebView2（選項 4 的 C）
- `WebView2\`：以 `expand <cab> -F:* <資料夾>` 解開微軟的 Fixed Version（微軟說明不要用檔案總管解）。
- 可攜版的設定檔把 `bundle.windows.webviewInstallMode` 設成 `fixedRuntime`、`path` 是 `WebView2`：Tauri 啟動時把 `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` 設成程式旁邊的 `WebView2\`，只用這一份，不找系統的。缺少或開不起來時，說明（重新解開 zip）並結束。
- 資料資料夾指定在 `data\EBWebView\`，畫面元件以 InPrivate 開啟（Tauri 視窗的 `incognito`）。POC 確認 InPrivate 之後還會寫哪些檔案，以及從隨身碟啟動要多久。
- **Windows 10 上的授權**：WebView2 120 版起，renderer 在 App Container 中執行，要讀得到 `WebView2\`。
  - 可攜版在 Windows 10 上啟動時，檢查 `WebView2\` 的 ACL 有沒有 ALL APPLICATION PACKAGES（`S-1-15-2-1`）與 ALL RESTRICTED APPLICATION PACKAGES（`S-1-15-2-2`）的讀取＋執行，並繼承到裡面的資料夾與檔案。
  - 沒有就以 `sandbox` 的 ACL 函式加上，效果與微軟說明的 `icacls <資料夾> /grant *S-1-15-2-2:(OI)(CI)(RX)` 與 `icacls <資料夾> /grant *S-1-15-2-1:(OI)(CI)(RX)` 相同。
  - 只在 NTFS、ReFS 上做；exFAT、FAT32 沒有 ACL。
  - 加不上時（與 worker 的授權寫不進去是同一種情形），說明原因與解法並結束。
  - 這兩個授權讓任何 app package 都能讀取與執行 `WebView2\` 裡微軟的檔案，不包括 `data\` 與其他檔案。
  - Windows 11 不用，也不加。
- **更新**：Fixed Version 不會自己更新。
  - 每次發行可攜版，都換成當時最新的 Fixed Version。
  - app 沒有新版時，至少每個月（WebView2 大約每四週一個大版本）重新發行一次可攜版，只換 WebView2；WebView2 有重大的安全修正時另外發。
  - 風險比一般瀏覽器低：畫面只顯示 app 自己的介面，CSP 擋下外部資源，不載入網頁，PDF 由 worker 畫成點陣圖。但 WebView2 仍會處理 app 的 HTML、CSS、JavaScript，以及從 PDF 讀出、當成純文字顯示的字串。
- **授權**：WebView2 Runtime 是微軟的軟體，照微軟的散布條款附在 zip 裡。實作的 PR 要確認條款的內容，把條款放進 zip，並在 `THIRD_PARTY_LICENSES` 說明它不是以 AGPL 授權。

### 檔案系統（選項 5 的 B）
- 啟動時檢查程式所在的磁碟：以 `GetVolumePathNameW` 找到它的根，再以 `GetVolumeInformationW` 取得檔案系統的名稱。
- 網路上的資料夾（`GetDriveTypeW` 是 `DRIVE_REMOTE`）不接受：worker 的隔離靠本機的檔案權限，Fixed Version 也不能從網路上的位置執行。說明（把資料夾放在電腦的硬碟或隨身碟上）並結束。
- **NTFS、ReFS**：worker 執行檔照現在的做法，啟動前把讀取＋執行權限授予容器；Windows 10 上再加 WebView2 的兩個授權；`data\` 與其他檔案沒有授權，容器碰不到。
- **授權可能寫不進去**（只在 NTFS、ReFS 上）：
  - 改 ACL 要 `WRITE_DAC`，檔案的擁有者有；以 Windows 格式化的 NTFS 隨身碟，預設只給 Authenticated Users「修改」，沒有 `WRITE_DAC`。
  - 容器的 SID 由名稱導出，每台電腦都一樣，所以授權寫進隨身碟上的 ACL 之後，換電腦也有效。
  - 但第一次執行的帳號不是 `pdf_worker.exe` 的擁有者時，授權寫不進去。例如在 A 電腦解壓縮後沒有執行，直接拿到 B 電腦執行；或用檔案總管把資料夾複製到另一個隨身碟（複製不帶明確的 ACE），再到別的電腦執行。
  - 現在遇到這種情形，`grant_read_execute` 當成成功，worker 開不了，畫面只有一般的錯誤。可攜版改成：授權失敗、ACL 裡也沒有這個容器的授權時，說明原因與解法（在解壓縮的那台電腦先執行一次，或用自己的帳號重新解壓縮），然後結束。WebView2 的兩個授權相同。
- **exFAT、FAT32**：也執行（選項 5 的 B）。這兩種沒有 ACL，不用、也不能授權。實作前 POC 先回答：
  1. worker（AppContainer，沒有任何 capability）從這種磁碟啟動得了嗎？Windows 10 上 WebView2 的 renderer 載入得了嗎？
  2. worker 讀寫得到同一個磁碟上的其他檔案嗎：`data\` 的設定與崩潰復原的日誌、主程式與 `pdf_worker.exe` 本身、隨身碟上別的檔案？電腦硬碟上的使用者檔案應該照樣碰不到（NTFS 的權限仍然擋），也要確認。
  3. 讀寫得到時，worker 被攻破就可能換掉隨身碟上的主程式，下次執行的就是沙盒外的別人的程式。負責人看了 POC 的結果，在「啟動時說明風險，使用者同意才繼續」與「說明並結束」之間選，或要另找做法；這一段照結果改寫。
- 其他檔案系統：說明並結束。

### 這台電腦上留下什麼
- 可攜版只在這台電腦上建立 worker 的 AppContainer profile（`%LOCALAPPDATA%\Packages\<名稱>\` 與 HKCU 的對應），**正常結束時刪除**（`sandbox::delete_app_container_profile`，在用這個容器的每一個行程都結束之後：每份文件的 worker、合併與轉換專用的 worker，以及之後加入、也用這個容器的行程）。當機時留下的，下次在這台電腦上正常結束時一起刪除。
- 畫面元件的資料在 `data\EBWebView\`（選項 4 的 C）。
- Windows 10 上 WebView2 的 renderer 在 App Container 中執行：POC 確認它在這台電腦上留下了什麼（例如 profile 或登錄機碼）。能刪的在正常結束時刪除，刪不了的寫進使用手冊。
- 所以工作卡的驗收「`%LOCALAPPDATA%` 中只留下 ADR 允許的東西」是：執行中只有那個 AppContainer profile（與 POC 找到、寫進使用手冊的 WebView2 的東西），正常結束之後沒有 app 的東西。
- Windows 自己記下的東西不在 app 的控制內，使用手冊寫明：檔案對話框最近用過的資料夾、執行過哪些程式、通知區域的圖示設定、SmartScreen 的紀錄等。

### 更新（ADR 0009 不變）
- 「檢查更新」照現在的做法：使用者按下才送一個請求，內容與安裝版相同，不說是可攜版。
- 有新版時，「前往下載頁…」開啟同一個發行頁；可攜版的說明改成：下載可攜版的 zip，解開到同一個資料夾取代舊的檔案（`data\` 不在 zip 裡，所以會留著）。
- 只換 WebView2 的重新發行也要有新的版號，「檢查更新」才看得到；版號怎麼訂由實作卡寫。
- 沒有自動更新。

### 建置與發行
- 發行的 workflow 與安裝檔一起建置：同一份前端與 `pdf_worker.exe`，主程式另外以可攜版的設定建置一次；zip 與 `.sha256`（格式與安裝檔的相同）一起上傳。zip 會超過 250 MB（安裝檔約 14 MB），GitHub Release 單一檔案的上限是 2 GiB，放得下。
- **WebView2**：發行的 workflow 下載固定版本的 Fixed Version `.cab`（版本與 SHA-256 寫在 repo 的檔案裡，只從微軟的網址下載），核對 SHA-256 與裡面 `msedgewebview2.exe` 的微軟簽章，再以 `expand` 解開，放進 zip。換版本的 PR 要 `needs-security-review`。下載只發生在 CI，app 照樣不連網。
- 與安裝檔相同：沒有簽章（DEC-04），從網路下載的 zip 解開後，第一次執行時 SmartScreen 會提醒；核對 SHA-256 的說明相同；散布時一併提供對應的原始碼（ADR 0011）。
- capability 與 CSP 不變。前端多知道一件事：這是不是可攜版（決定顯示哪些項目、怎麼說明更新），沒有新的命令參數，也沒有路徑。

### 測試
- 主行程：資料資料夾的位置；資料夾不能寫入；網路上的資料夾；不是 NTFS、ReFS、exFAT、FAT32 的磁碟；授權寫不進去、ACL 裡也沒有容器的授權時說明並結束；Windows 10 上加上 WebView2 的兩個授權，已經有時不重複加；不讀 `PDF_READER_DATA_DIR`；不提供的項目被拒絕（全文索引、簽名圖片、記住密碼、設為預設）；最近開啟的檔案預設關閉；正常結束時刪除 profile，當機留下的在下次結束時刪除。exFAT、FAT32 照 POC 選定的做法測。
- E2E（工作卡）：每次把可攜版解壓縮到新的暫存資料夾，`data\` 就在那裡；以可攜版開檔、編輯、存檔、結束；結束後 `%LOCALAPPDATA%` 沒有 app 的東西，profile 已刪除；用的是 `WebView2\` 裡的 WebView2，不是系統的。另外一次：安裝版與可攜版同時開著，各自開自己的檔案。
- Windows 10 22H2：CI 沒有這種 runner，由負責人在發行前用實機或虛擬機檢查（manual-checks.md 加一節）。
- 發行腳本的測試（`scripts/release/*.test.mjs`）：zip 的內容（包括 `WebView2\` 與它的授權條款）與 `.sha256`。

### POC（實作前，不通過就不實作）
1. 可攜版的建置（另一個 `identifier`、`fixedRuntime`）：單一執行個體與安裝版分開；用的是 `WebView2\` 裡的 WebView2；`check-security-config.mjs` 通過。
2. 在沒有裝過 app 的 Windows 11 上，從 NTFS 的隨身碟執行：開檔、編輯、存檔、結束；比對執行前後的 `%LOCALAPPDATA%`、`%TEMP%` 與 HKCU，結束後只剩 Windows 自己的紀錄。也在 ReFS 上跑一次。授權寫不進去的兩種情形（在 A 電腦解壓縮後沒有執行，直接拿到 B 電腦執行；以檔案總管把資料夾複製到另一個隨身碟，再到別的電腦執行）都要說明原因並結束。
3. 在沒有裝 WebView2、沒有網路的 Windows 10 22H2 上：第一次執行加上 WebView2 的兩個授權；開檔、編輯、存檔、結束；比對執行前後，包括 WebView2 自己留下的。
4. 在 exFAT 與 FAT32 的隨身碟上：回答「檔案系統」的三個問題。
5. WebView2 的資料在隨身碟、以 InPrivate 開啟：寫了哪些檔案；USB 2.0 與 3.0 的隨身碟啟動要多久；每次重新建立 profile 要多久。
- 2 到 4 要乾淨的 Windows 11 與 Windows 10 22H2：另一台電腦或虛擬機。開發機是 Windows 11 家用版，沒有 Hyper-V 與 Windows 沙箱，用虛擬機要另外安裝（例如 VirtualBox），由負責人決定。

## 後果
- 換來：
  - 不能安裝（沒有系統管理員權限）的電腦也能用，也可以放在隨身碟帶著走；
  - 沒有 WebView2、不能連網的電腦，以及還沒升級的 Windows 10 22H2，也能用；
  - 隨身碟不用格式化（exFAT、FAT32 的隔離照 POC 的結果處理）；
  - 正常結束之後，這台電腦上沒有 app 的資料（Windows 自己的紀錄除外）；
  - 安裝版的行為完全不變。
- 付出：
  - 另一個建置：CI 與發行多建置一次主程式，發行多兩個檔案；
  - zip 超過 250 MB；
  - WebView2 的安全更新要我們跟著發行，至少每個月一次；
  - 要多測一種系統（Windows 10 22H2），CI 沒有，靠負責人用實機或虛擬機檢查；
  - Windows 10 上 `WebView2\` 留著兩條給所有 app package 的授權（讀取＋執行）；
  - 資料的安全只靠隨身碟本身：拿到隨身碟的人讀得到設定、機敏文件的檔名、崩潰復原的日誌（未存檔的編輯與完整路徑），開了最近開啟的檔案時還有路徑。使用手冊建議以 BitLocker To Go 加密隨身碟；
  - 程式在使用者可以寫入的資料夾：拿到隨身碟的人可以換掉程式（安裝版在 Program Files，一般權限的程式換不掉 `pdf_worker.exe`）。在 exFAT、FAT32 上，worker 被攻破時也可能換掉程式（POC 之前不知道）。使用手冊寫明只在自己保管的隨身碟上用，懷疑時重新下載並核對 SHA-256；
  - 隨身碟上的 `pdf_worker.exe` 留著一條給這個容器的授權（讀取＋執行）；第一次執行的帳號改不了 ACL 時（例如解壓縮後沒有執行就換了電腦），可攜版不能用，要照說明重新解壓縮；
  - 少了全文索引、簽名圖片、記住密碼與設為預設；
  - 不能放在網路上的資料夾；
  - 換電腦後磁碟代號不同，崩潰復原（與開啟時的最近開啟的檔案）以完整路徑找，會找不到；
  - 從隨身碟執行比較慢。
- 擋不到的：Windows 自己記下的東西；隨身碟上的檔案被別人讀走或換掉；Windows 10 本身沒有修補的漏洞。
- 不改變：不連網（檢查更新只在使用者按下時，ADR 0009；下載 WebView2 只在 CI）；零遙測；worker 的沙盒（AppContainer 換了名稱，仍然沒有任何 capability）；前端拿不到路徑；capability 與 CSP；安裝版，它照舊只支援 Windows 11。
- 規格 §1（可攜版，中英文版）在接受後另開文件 PR 更新（這張卡只動 `docs/adr/`）。`local-data.md`、`packaging.md`、`manual-checks.md`、`docs/user/01-install-and-start.md` 與 `05-security-and-privacy.md` 由實作的 PR 寫。
- 之後若要改變這個決定，代價是：
  - 改回系統的 WebView2（選項 4 的 A）：zip 變小，但沒有 WebView2 的電腦不能用；
  - 不再支援 Windows 10（例如微軟不再更新那裡的 WebView2）：先在使用手冊與發行說明公告；
  - 可攜版也要全文索引或簽名圖片：要一把不靠 DPAPI 的金鑰（例如由使用者的密碼導出），另寫 ADR；
  - 改成標記檔（選項 1 的 B）：安裝版要多讀一個輸入，單一執行個體要另外分開。
