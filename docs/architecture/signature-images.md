# 重複使用的簽名圖片（B6-02）

工作卡 [#587](https://github.com/winner0988/Pdf-reader/issues/587)（B6-02）；規格 §6。

## 概要

使用者可建立重複使用的簽名圖片（手繪、輸入姓名或匯入現成圖檔），並加密保存在這台電腦上。之後開啟任何文件時，點一下即可將已存簽名作為圖片印章蓋在頁面上，並可自由移動、縮放與旋轉。

**這不是數位簽章（Digital Signature，ADR 0014）**：放上文件的是標準 PDF 圖片印章註解（`Stamp`），僅具備視覺上的外觀，不含密碼學憑證，也不能證明是誰簽署的。UI 明確以說明提示此差異。

## 本機存放與 DPAPI 加密

簽名屬於個人的敏感筆跡與印鑑外觀，依負責人決定（#587 方案 A）採用本機加密保存：

- **存放路徑**：`%LOCALAPPDATA%\io.github.winner0988.pdfreader\signatures\<id>.sig`（見 [local-data.md](local-data.md)）。
- **加密方式**：使用 Windows Data Protection API（DPAPI，`CryptProtectData` / `CryptUnprotectData`），額外附加 entropy `b"Pdf-reader signature image (B6-02)"`。
- **安全性與邊界**：
  - 加密金鑰綁定目前的 Windows 使用者帳號，同一台電腦的其他 Windows 帳號或未登入者無法解密。
  - 只有主行程負責檔案讀寫與加解密；前端 WebView2 只能拿到型別化的 IPC 資訊與預覽用記憶體資料，永遠接觸不到磁碟實體路徑。
  - 解密後的明文 PNG 位元組在主行程使用 `zeroize::Zeroizing<Vec<u8>>` 包裝，在向圖片庫註冊完成後立即從記憶體中抹除。
  - 檔案寫入透過 `local_data::write`，先寫入同目錄暫存檔再不可分割地更名取代，避免斷電或當機時留下損毀或未加密的暫存檔。

## 上限

定義於 `crates/ipc_contract/src/limits.rs`：
- `MAX_SIGNATURE_IMAGES = 10`：最多保存 10 個簽名圖片。達到上限時前端停用新增按鈕，提示先刪除既有簽名。
- `MAX_SIGNATURE_FILE_BYTES = 2 * 1024 * 1024`（2 MiB）：單一 `.sig` 檔案讀取大小上限，防止惡意超大檔案造成記憶體耗盡。

## IPC 指令（7 個）

主行程提供 7 個專屬 Tauri 命令（`src-tauri/src/commands.rs`），列入 `docs/security/attack-surface.md` §1.13：

1. `get_signature_images()` -> `Vec<SignatureImage>`：
   列出所有已存簽名的清單（包含 `id`、`width`、`height`）。
2. `get_signature_image_png(args: SignatureImageArgs)` -> `Vec<u8>`：
   讀取並解密指定簽名的 PNG 位元組，供前端在對話框中繪製預覽縮圖。
3. `add_signature_image(args: AddSignatureImageArgs)` -> `Vec<SignatureImage>`：
   接收手繪或文字產生的 PNG，驗證標頭與尺寸後，DPAPI 加密寫入新的 `<id>.sig`。
4. `import_signature_image()` -> `Option<Vec<SignatureImage>>`：
   開啟系統檔案選擇對話框，將使用者選取的圖片交給 sandboxed worker 解碼、去除所有中繼資料（EXIF 等）並標準化為乾淨的 PNG，再加密保存。使用者取消時回傳 `None`。
5. `remove_signature_image(args: SignatureImageArgs)` -> `Vec<SignatureImage>`：
   刪除單一簽名檔案 `<id>.sig`，回傳剩餘簽名清單。
6. `remove_all_signature_images()` -> `Vec<SignatureImage>`：
   刪除 `signatures\` 資料夾與其中所有簽名檔案，供設定頁「全部刪除」使用。
7. `prepare_signature_stamp(args: PrepareSignatureStampArgs)` -> `StampImageInfo`：
   解密指定簽名至 `Zeroizing` 緩衝區，呼叫 `Documents::add_stamp_picture` 登記於目前文件的圖片庫中，回傳可供放置的印章圖片資訊。

## 蓋在文件上與崩潰復原

簽名放上頁面後，沿用標準自訂圖片印章（[annotations.md](annotations.md)）：
- 透過 `Edit::AddImageStamp` 套用編輯，可以復原（Undo）與重做（Redo）。
- 崩潰復原日誌（`recovery\`）僅包含該文件已套用編輯所用到的圖片點陣資料，遵守日誌的大小與十六進位驗證規則（[crash-recovery.md](crash-recovery.md)）。
- 存檔後簽名成為 PDF 頁面內容中無中繼資料的圖片 XObject，符合 PDF 標準，在任何標準 PDF 閱讀器均可正常檢視。

## 用於數位簽章外觀（#744）

除了自訂圖片印章之外，已存簽名也可在「簽署文件」對話框（[signing.md](signing.md)）中搭配「顯示在目前這一頁的角落」選為可見簽章外觀的簽名圖片（不適用於不可見簽章）：

- 簽署時主行程將選中的 `.sig` 解密進 `Zeroizing<Vec<u8>>`，經 IPC（`PrepareSignature`）只在記憶體中傳給目前文件的 worker，不寫任何暫存 PNG 檔，送出與丟棄後立即清除。
- worker 先驗證 PNG 標頭、尺寸與完整區塊，再以 MuPDF 重解碼並以像素重建圖片 XObject（`/I`）放進簽章的 `/AP /N` 外觀串流，不會將原檔中繼資料或路徑寫入已簽署的 PDF。
