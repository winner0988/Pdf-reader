# Glossary (English)

Fixed English terminology for PDF Reader interface translations, aligned with `CONTEXT.md` and `docs/ux/screen-map.md`.

## Core Concepts (CONTEXT.md)

| Traditional Chinese | English | Notes / Context |
|---|---|---|
| 表單腳本沙盒 | Form Script Sandbox | Sandboxed execution environment for form scripts (ADR 0021). |
| 信任（單一文件） | Trusting a Document | Deprecated / no longer used (ADR 0022). |
| 文件識別碼 | Document ID | Persistent application-generated identifier in PDF metadata (ADR 0010). Distinct from runtime `DocumentId`. |
| 密碼記憶 | Remembered Password | Encrypted passwords stored in Windows Credential Manager (ADR 0017). |
| 安全遮蔽 | Redaction | Permanent removal of content, images, and text from the file (ADR 0016). Distinct from black overlay boxes. |
| 機敏文件 | Sensitive Document | User-marked document enabling OS-level leak prevention (ADR 0004, ADR 0023). |
| 批次處理背景服務 | Batch Processing Background Service | Background task queue executing with system tray icon (ADR 0005). |
| 掃描頁 | Scanned Page | Image-only page with no text layer (ADR 0015). |
| 辨識文字 | Recognised Text | Ephemeral text recognized by OCR from scanned pages (ADR 0015). |
| 文字方塊 | Text Box | FreeText annotation added by user with embedded font subset (ADR 0018). |
| 簽署者受信任 | Trusted Signer | Signer certificate chaining to a trusted Windows root CA (ADR 0014). |
| 文件代號 | DocumentId | Opaque runtime identifier representing current open document state (ADR 0008). |
| 編輯 | Edit | Typed command sent from frontend to main process and worker (ADR 0013). |
| 來源檔 | Source File | Untrusted external PDF imported into document (B2-06, ADR 0031). |
| 主動內容 | Active Content | JavaScript, launch actions, or remote URIs blocked by security scanner (ADR 0001). |
| 全文索引 | Text Index | Cross-document local search index encrypted with DPAPI (ADR 0026). Distinct from single-document text cache (#242). |

## UI Actions and Features

| Traditional Chinese | English | Notes |
|---|---|---|
| 介面語言 | Interface Language | Setting title: "介面語言 Language" |
| 跟隨 Windows | Follow Windows | Language option following OS display language |
| 繁體中文 | Traditional Chinese | Language option name in Chinese |
| 重新啟動 PDF Reader 後生效 | Takes effect after restarting PDF Reader | Restart notice when changing language |
| 隱私匯出 | Privacy Export | Clearing all metadata and document IDs |
| 頁碼與 Bates 編號 | Page Numbers and Bates Numbering | Page numbering and Bates stamping dialog |
| 浮水印 | Watermark | Text or image watermark dialog |
| 安全遮蔽 | Redaction | Panel and dialogs for marking and applying redaction |
| 找出個資 | Find Personal Data | Searching for PII (national IDs, credit cards, phones, etc.) |
| 另存為可搜尋的 PDF | Save as Searchable PDF | Exporting OCR text as invisible layer |
| 數位簽章 | Digital Signatures | Digital signature verification and details |
| 簽署文件 | Sign Document | Applying digital signature with certificate |
| 扁平化表單 | Flatten Form | Baking form field values into page content |
| 匯出註解摘要 | Export Annotation Summary | Plain text or Markdown summary of annotations |
| 匯出註解（XFDF） | Export Annotations (XFDF) | Standard XFDF export of annotations |
| 視覺比較 | Visual Compare | Side-by-side rendering comparison |
| 比較文字 | Compare Text | Text diff comparison between two open tabs |
| 裁切頁面 | Crop Pages | Visual page crop dialog |
| 取代頁面 | Replace Pages | Replacing pages with pages from another file |
| 旋轉頁面 | Rotate Pages | Rotating selected pages |
| 刪除頁面 | Delete Pages | Deleting selected pages |
| 插入頁面 | Insert Pages | Inserting pages from file or scanner |
| 命令面板 | Command Palette | Keyboard command palette (Ctrl+K) |
| 最近開啟的檔案 | Recent Files | Local recent files list |
| 不記錄此檔案 | Do not record this file | Exclusion setting for specific open document |
| 檢查更新 | Check for Updates | Manual update check with GitHub release API |
