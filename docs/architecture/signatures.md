# 數位簽章驗證（B2-14）

對應 [ADR 0014](../adr/0014-signature-verification.md)（負責人於 2026-09-30 接受選項 A）、工作卡 [#170](https://github.com/winner0988/Pdf-reader/issues/170)、規格 §3「本地數位簽章」的**驗證**部分。畫面與文字見 [screen-map.md](../ux/screen-map.md)「數位簽章（B2-14）」。

## 範圍

- 開啟已簽章的文件時，告訴使用者每個簽章的狀態：簽章是否成立、簽署之後檔案有沒有變更、簽署者是誰、憑證是否受信任。
- **完全離線**：不查 OCSP／CRL、不下載中繼憑證或信任清單；worker 的 AppContainer 本來就沒有網路。
- **不做**：簽署文件、載入 `.pfx`（另開卡與 ADR）、時間戳記（RFC 3161）、內嵌撤銷資訊（DSS）的解讀（另開卡）。

## 流程

1. 文件載入、沒有未儲存的變更時，前端呼叫 `get_signatures(doc)`（背景；不擋開檔與渲染）。簽章是**檔案**的：有未儲存的變更時不重新驗證，儲存之後依新的檔案重新驗證。
2. 主行程把它交給該文件自己的 worker（`VerifySignatures`），只收到結果：`SignatureReport`，每個值在使用前通過 `Validate`（筆數、文字長度與乾淨程度、「沒有成立的簽章不會有簽署者」）。主行程不解析簽章資料。
3. worker 在 `serve` 迴圈裡處理，用的是它保存的**檔案位元組**（`originals`：開啟時或上次存檔時的檔案），不是編輯後的文件：簽章涵蓋的是位元組。

## worker 做什麼

### 找出簽章欄位（`engine/signatures.rs`）

從 `/AcroForm /Fields` 往下走（深度優先、依表單的順序）：欄位的 `/FT` 是自己的或是上層的（`/Sig`），而且自己有 `/V` 字典，就是一個簽章。同一個 `/V` 物件被好幾個欄位指到只算一個；欄位彼此指回自己（`/Kids` 迴圈）只走一次。最多看 10,000 個欄位、深度 64、驗證 `MAX_SIGNATURES` = 100 個簽章；超過時回報 `truncated`，畫面會說明沒有全部驗證。

每個簽章只是**讀出來**：`/T`（欄位名稱）、`/SubFilter`、`/ByteRange`、`/M`（簽署時間）、`/Contents`，以及認證簽章的 DocMDP。每一項各自讀、讀不到就是沒有，一個壞掉的欄位不會讓別的簽章也看不到。`/Contents` 最後才複製（見下方第 3、5 步）。

DocMDP 只認 `/Perms /DocMDP` 指到的那個簽章（規格規定只有它能認證文件），它的 `/Reference` 裡的 `/TransformParams /P`（沒寫是 2）：1＝不允許任何變更、2＝只允許填表單與簽署、3＝再加註解。

### 檢查一個簽章（`signatures.rs`），依序

| 步驟 | 檢查 | 不符時 |
|---|---|---|
| 1 | `/SubFilter` 是 `adbe.pkcs7.detached` 或 `ETSI.CAdES.detached`（兩者都是對檔案位元組的 CMS 分離簽章） | 無法驗證（格式不支援）：舊的 `adbe.pkcs7.sha1`、`adbe.x509.rsa_sha1`、文件時間戳記 `ETSI.RFC3161` 等 |
| 2 | `/ByteRange` 是四個整數：第一段從 0 開始且不是空的，兩段之間至少放得下 `<>`，第二段不是空的而且在檔案之內 | 無效 |
| 3 | 兩段之間（簽章的十六進位文字）不超過 `2 × MAX_SIGNATURE_CONTENTS_BYTES + 2`（2 MiB 多一點），在複製或解碼任何東西**之前**檢查 | 無法驗證（太大） |
| 4 | 兩段之間剛好是 `<` 十六進位數字（空白略過）`>`，沒有別的東西 | 無效 |
| 5 | 表單字典裡的 `/Contents` 與那段十六進位解出的位元組**完全一樣** | 無效 |
| 6 | 簽章資料開頭是 DER 的 SEQUENCE，長度是定長（短形式，或最多四個長度位元組）；後面的補零不送去驗證 | BER 不定長度：無法驗證（格式）；被截斷：無效 |
| 7 | Windows 驗證 CMS（見下方） | 見下方的對應 |
| 8 | 第二段之後的位元組都是空白 | 不是：「簽署之後有變更」（簽章仍成立）。檔尾多一個換行不算變更 |

第 4、5 步把「檔案裡被簽章留空的那一段」和「表單看到的簽章」綁在一起：簽章的 `/Contents` 不在簽章涵蓋的範圍裡，如果不檢查，攻擊者可以把那一段換成別的內容（例如插入 PDF 語法），或讓字典的 `/Contents` 指到別的字串，而簽章本身仍然成立（signature wrapping）。測試 `a_signature_the_form_no_longer_has_is_not_the_one_the_range_leaves_out` 用附加更新替換簽章字典來驗證這一步；拿掉檢查，它就會變成「簽署之後有變更」而失敗。

### Windows CryptoAPI（`crypt.rs`，唯一的 `unsafe`）

- `CryptVerifyDetachedMessageSignature`：以簽章資料驗證兩段內容。簽署者的憑證必須在簽章資料裡。
- 憑證鏈：`CertGetCertificateChain`，加上簽章資料帶的憑證（`CryptGetMessageCertificates`，簽署者的中繼 CA 通常在裡面），旗標 `CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL | CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE`：只用這台電腦已有的資料，不取回任何網址，也不自動更新信任的根憑證；沒有要求撤銷檢查。鏈的 `dwErrorStatus` 為 0（沒有任何問題：根憑證受信任、鏈完整、期限內）才算「簽署者受信任」；自簽、根憑證不受信任、鏈不完整、憑證過期都是「無法確認簽署者」。
- 簽署者名稱：`CertGetNameStringW`（`CERT_NAME_SIMPLE_DISPLAY_TYPE`）；之後清除控制字元與不可見的格式字元（`clean_display_text`），最多 `MAX_SIGNATURE_TEXT_BYTES`。
- 錯誤碼的對應：雜湊或簽章值不符（`CRYPT_E_HASH_VALUE`、`NTE_BAD_SIGNATURE`、`STATUS_INVALID_SIGNATURE`）、訊息壞掉（ASN.1 錯誤、`CRYPT_E_BAD_MSG` 等）：**無效**；不認得的演算法（`CRYPT_E_UNKNOWN_ALGO`、`NTE_BAD_ALGID`、`STATUS_HASH_NOT_SUPPORTED`）：無法驗證（演算法）；找不到簽署者的憑證或其他沒見過的錯誤：無法驗證（格式）。不是 Windows 時一律「無法驗證（這個系統）」。
- 憑證、簽章與位元組都來自不受信任的檔案，由 Windows 的 C 程式碼解析：在 worker 的沙盒裡，沙盒沒有放寬。`unsafe` 都在 `crypt.rs`，每一段有 SAFETY 說明，配置出來的 context 與 store 都用 `Drop` 釋放。

## 狀態

| `status` | `signerTrusted` | 條件 | 畫面（signatures.status.*） |
|---|---|---|---|
| `valid` | 是 | 簽章成立、涵蓋整個檔案、憑證鏈到這台電腦的 Windows 信任的根憑證 | 簽章有效，簽署者受信任（綠色） |
| `valid` | 否 | 同上，但無法確認簽署者（例如自簽，語料就是） | 簽章有效，但無法確認簽署者 |
| `changedAfterSigning` | 是／否 | 簽章成立，但檔案在簽署之後有更多位元組（增量更新，也可能只是別人又加的簽章） | 簽署之後文件有變更 |
| `invalid` | 否 | 見檢查的步驟 2、4、5、6 與 Windows 的「無效」 | 簽章無效（紅色） |
| `unverifiable` | 否 | 步驟 1、3、6 或 Windows 的「無法驗證」，`reason` 說明原因 | 無法驗證 |

- 「簽章有效」從不說成「可信任」；撤銷狀態沒有檢查，畫面一律說明。
- 簽署時間是簽字典的 `/M`：沒有時間戳記，只是簽署者自己的說法，畫面這樣標示。格式是 `D:YYYYMMDDHHmmSSOHH'mm'`，至少要有分鐘，否則不顯示；顯示成 `2026-09-24 12:00:00 UTC+08:00`。
- 認證簽章（DocMDP）只在畫面上轉成文字；`P` 為 1（不允許任何變更）而且文件在簽署之後有變更時多一行警告。P 為 2 或 3 時**不判斷**簽署之後的變更是否在允許的範圍內（要比對增量更新改了哪些物件，這一版不做），只說簽署者允許什麼。

## 上限

| 項目 | 上限 |
|---|---|
| 驗證的簽章數 | `MAX_SIGNATURES` = 100 |
| 走過的表單欄位 | 10,000；深度 64 |
| 簽章資料（`/Contents`） | `MAX_SIGNATURE_CONTENTS_BYTES` = 1 MiB（檔案裡的十六進位文字是兩倍） |
| 簽署者名稱、欄位名稱 | `MAX_SIGNATURE_TEXT_BYTES` = 256 bytes |
| 一個簽章欄位的複製 | 每個簽章最多複製一次 `/Contents`，而且在範圍確定放得下之後 |

請求用一般的逾時，逾時即終止並重啟 worker（與其他請求相同）。

## 測試

- `crates/ipc_contract`：`a_signature_report_is_bounded_and_consistent`（筆數、原因只出現在無法驗證的簽章、沒有成立的簽章不會有簽署者、文字長度與乾淨程度）；模糊測試的種子有新的請求與回應。
- `crates/pdf_worker/src/signatures.rs`：範圍、十六進位文字、DER 長度、檔尾空白、認證等級、簽署時間的單元測試。
- `crates/pdf_worker/src/crypt.rs`：「根憑證存放區裡的根憑證，憑證鏈判斷為受信任」——證明判斷「受信任」的程式不是只會說不。
- `crates/pdf_worker/tests/signatures.rs`（真正的 worker、真正的沙盒）：
  - 語料的兩個簽章檔（`benign/signed.pdf`、`benign/signed-docmdp-p1.pdf`）：簽章成立、簽署者是語料的測試憑證、**不受信任**（自簽）、簽署時間、DocMDP P=1；
  - 改動簽章範圍內的一個位元組、改動簽章值本身：無效，而且沒有簽署者；
  - 簽章之後附加增量更新：簽署之後有變更、簽章仍成立；檔尾多換行：沒有變更；
  - 其他 `/SubFilter`：無法驗證（格式）；超過 2 MiB 的十六進位文字：無法驗證（太大）；
  - 範圍留空的不是表單看到的 `/Contents`：無效（上述的綁定）；
  - 沒有簽章的文件與有其他欄位的表單：沒有簽章；101 個欄位：只驗證 100 個並標示 `truncated`；欄位彼此指回自己、兩個欄位指到同一個簽章、欄位的類型來自上層。
- 前端：`summary.test.ts`、`useSignatures.test.tsx`（何時驗證、儲存之後重驗、別的檔案不吃舊答案）、`Signatures.test.tsx`（提示列的顏色與句子、面板的內容與焦點、與已封鎖內容明細輪流開啟）。
- E2E：`tests/e2e/signatures.spec.ts`（真正的 app）：兩個語料檔的提示列與面板、改動過的檔案顯示無效、沒有簽章的文件沒有提示列。

## 已知限制

- 沒有受信任簽署者的正向樣本：語料只有自簽的憑證，要有受信任的簽章就需要真的 CA 簽發的檔案（不能放進 repo）。「受信任」的判斷以 Windows 自己的根憑證存放區測試。
- 簽署者名稱取自憑證，不是簽章字典的 `/Name`（後者是檔案作者自己寫的，不驗證）。
- 一個文件有好幾個簽章時，先簽的那個在後面的簽章加進來之後會顯示「簽署之後文件有變更」（嚴格說是對的：檔案多了位元組），說明裡提到「也可能只是又加了別人的簽章」。
- 沒有判斷簽署之後的變更是否在 DocMDP 允許的範圍內（只有 P=1 加上任何變更會警告）。
- 加密而且有簽章的文件：簽章的 `/Contents` 不加密；若 MuPDF 讀到的字串與檔案裡的不同，這個簽章會被判為無效（比判為有效安全），沒有專門的樣本測試。
- 只有 Windows（ADR 0014 的代價）；換成 RustCrypto 只影響 `crypt.rs`。
