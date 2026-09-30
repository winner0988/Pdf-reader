# ADR 0014：數位簽章在 worker 中以 Windows CryptoAPI 離線驗證

## 狀態
已接受

（負責人於 2026-09-30 接受選項 A〔#103〕。工作卡 #100〔B2-11〕。POC：`crates/pdf_worker/tests/signature_poc.rs`、`crates/sandbox/tests/sandbox.rs` 的 `root_certificates_are_readable`。原本的卡片寫作 ADR 0015；因為 ADR 編號必須連續、而本條比 OCR 的 ADR 先完成，改為 0014，OCR 改為 0015。）

## 背景
- 規格 §3「本地數位簽章」：驗證完全離線。本 ADR 只處理**驗證**；簽署與載入 `.pfx` 另寫 ADR。
- 已簽章的 PDF 很常見（合約、公文）。目前 app 不顯示簽章狀態：使用者看不出文件在簽章後有沒有被修改。
- 簽章資料（簽章字典、CMS `SignedData`、憑證）都來自不受信任的 PDF。依 ADR 0008，解析必須在沙盒中的 worker 進行。
- 語料有兩個簽章樣本（QA-04）：`benign/signed.pdf`、`benign/signed-docmdp-p1.pdf`。它們以 `adbe.pkcs7.detached`、SHA-256、RSA-2048 簽署，使用語料自己的自簽測試憑證。
- 不連網：
  - 不查 OCSP／CRL，不下載中繼憑證；
  - 不下載 Adobe 的 AATL 信任清單；
  - worker 的 AppContainer 本來就沒有網路。
- POC 的發現：
  - Windows CryptoAPI（`crypt32.dll`）驗證兩個樣本：數學上有效、簽署者正確、根憑證不受信任（自簽）；
  - 改動簽章範圍內的一個位元組：驗證失敗；
  - 簽章後以增量更新修改：簽章對「簽署時的內容」仍然有效，但 `/ByteRange` 不再涵蓋整個檔案，可以據此告訴使用者文件在簽章後有變更；
  - `crypt32.dll` 只依賴核心系統 DLL（不需要 `user32`，所以在停用 win32k 的 worker 中也能載入）；
  - 在真正的沙盒（AppContainer、低完整性、停用 win32k）中，目前使用者與電腦的根憑證存放區都讀得到（本機各 58 張）。

## 選項

| 選項 | 內容 | 代價 |
|---|---|---|
| **A. Windows CryptoAPI（建議）** | `CryptVerifyDetachedMessageSignature` 驗證 CMS；`CertGetCertificateChain`（`CACHE_ONLY`）對 Windows 信任的根憑證建立憑證鏈 | worker 多一個 `unsafe` FFI 模組；只能在 Windows（ADR 0007：首發平台就是 Windows）；CMS 由系統的 C 程式碼解析，在沙盒中執行 |
| B. RustCrypto（`cms`、`x509-cert`、`rsa`、`sha2`） | 純 Rust、記憶體安全的解析 | `rsa` 有尚未修補的 RUSTSEC-2023-0071（私鑰運算的時序側通道；驗證不受影響，但 `cargo deny` 需要例外）；憑證鏈與根憑證要自己處理；新增十幾個套件 |
| C. MuPDF 內建的驗證 | 需要以 OpenSSL 建置 MuPDF | 新增大型 C 依賴與建置流程 |
| D. `ring` 驗證＋RustCrypto 解析 | 避開 `rsa` 的公告 | `ring` 含 C 與組合語言，憑證鏈仍要自己處理 |

## 決定

### 在哪裡、用什麼驗證
- 在**該文件的 worker** 中驗證（沙盒內），使用 **Windows CryptoAPI**。
- FFI 集中在 `pdf_worker` 的一個模組（與 `handle.rs` 相同的做法），每一段 `unsafe` 都寫 SAFETY 說明。
- 主行程只收到驗證結果，不解析簽章資料。

### 驗證的步驟（每個簽章欄位）
1. 從簽章字典讀出 `/ByteRange` 與 `/Contents`：
   - 檢查四個數字在檔案範圍內、互不重疊；
   - 第一段從 0 開始；
   - 兩段之間恰好是 `/Contents` 的字串。
2. `/Contents` 只取 DER 的長度，後面的補零不送進驗證。
3. 以 `CryptVerifyDetachedMessageSignature` 驗證兩段內容與 CMS。
4. 以 `CertGetCertificateChain` 建立憑證鏈，帶 `CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL`：只用本機已有的資料，絕不擷取網址。
5. `/ByteRange` 的結尾不是檔案結尾：簽章之後有增量更新。

### 顯示給使用者的狀態

| 狀態 | 條件 |
|---|---|
| 簽章有效，簽署者受信任 | 驗證成功，憑證鏈到 Windows 信任的根憑證，涵蓋整個檔案 |
| 簽章有效，但無法確認簽署者 | 驗證成功，但根憑證不受信任（例如自簽，語料就是這種） |
| 簽章後有變更 | 驗證成功，但之後有增量更新；說明「簽章對簽署時的內容仍然有效」 |
| 簽章無效 | 驗證失敗：文件在簽章範圍內被修改，或簽章本身不符 |
| 無法驗證 | 不支援的格式或演算法（例如舊的 `adbe.x509.rsa_sha1`、未知的 `/SubFilter`），說明原因 |

- 一律說明「撤銷狀態未檢查（離線）」，除非文件內嵌了撤銷資訊（DSS、`adbe-revocationInfoArchival`）。內嵌撤銷資訊的解讀另開卡。
- 簽署時間：沒有時間戳記（RFC 3161）時，顯示為「簽署者聲稱的時間」。
- DocMDP（認證簽章，例如語料的 P=1「不允許任何變更」）：顯示允許的變更，簽章後的變更不符合時警告。
- 「數學上有效」永遠不說成「可信任」。

### 上限
- 每份文件最多驗證 100 個簽章欄位。
- `/Contents` 最多 1 MB。
- 驗證在背景進行，不阻擋開檔與渲染。

## 後果
- 換來：
  - 不新增任何套件（`windows-sys` 已是依賴，只多一個功能）；
  - 使用 Windows 維護的密碼學與根憑證清單；
  - 在沙盒中完成，不連網。
- 付出：
  - worker 多一個 `unsafe` 模組，需要安全審查；
  - CMS 由系統的 C 程式碼解析（在沙盒中）；
  - 只能在 Windows 上使用，移植到其他平台時要改用選項 B。
- 之後若要改變這個決定：驗證的介面（輸入兩段內容與 CMS，輸出狀態）與實作分開，換成 RustCrypto 只影響 worker 內的一個模組。
