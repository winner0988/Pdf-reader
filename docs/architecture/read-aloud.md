# 朗讀架構與安全設計（Read Aloud）

工作卡 B6-14（Issue #599，規格 §6）。以 Windows 內建的離線語音朗讀目前頁面或選取的文字，完全不連網、不寫檔、不留紀錄。

## 1. 選用的 Windows API（方案 A）

採用 WinRT 的 **`Windows.Media.SpeechSynthesis.SpeechSynthesizer`**（語音合成）搭配 **`Windows.Media.Playback.MediaPlayer`**（記憶體串流播放），只啟用既有 `windows` 0.62 crate 的 6 個功能（`Foundation`、`Foundation_Collections`、`Media_Core`、`Media_Playback`、`Media_SpeechSynthesis`、`Storage_Streams`），不新增任何第三方 crate，`Cargo.lock` 保持不變。

選擇方案 A 而非 SAPI 5（`ISpVoice`）的理由：
- **僅用本機已安裝的離線語音**：依 Microsoft Learn 官方文件（[`SpeechSynthesizer.AllVoices`](https://learn.microsoft.com/en-us/uwp/api/windows.media.speechsynthesis.speechsynthesizer.allvoices)），`AllVoices` 只列舉系統上已經安裝且經 Microsoft 簽署的本機 OneCore 語音套件，不會存取雲端語音或自動下載語音。
- **原生的詞界（Word Boundary）中繼資料**：開啟 `SpeechSynthesizerOptions.IncludeWordBoundaryMetadata` 後，`SpeechSynthesisStream` 會實作 `ITimedMetadataTrackProvider`，交給 `MediaSource::CreateFromStream` 與 `MediaPlaybackItem` 時，`SpeechWord` 軌道的 `SpeechCue` 會即時回報每個詞在輸入文字中的 `StartPositionInInput` 與 `EndPositionInInput`。
- **原生記憶體串流播放**：不需建立任何暫存 `.wav` 檔。
- **不引入網路 DLL**：`src-tauri/tests/binary_imports.rs` 確認啟用這 6 個功能後，`pdf-reader.exe` 仍未匯入任何禁止的網路 DLL。

## 2. 執行緒與可測試架構（`src-tauri/src/speech.rs`）

- **`SpeechBackend` trait**：抽象出 `list_voices`、`speak`、`pause`、`resume`、`stop`。正式執行使用 `WinRtSpeechBackend`，單元測試注入假引擎（`FakeBackend`），測試時不發出任何聲音。
- **獨立 COM 執行緒（`pdf-reader-speech`）**：`SpeechService` 啟動一條專用執行緒，在該執行緒以 `CoInitializeEx(None, COINIT_MULTITHREADED)` 初始化 COM（配對 RAII `CoUninitialize`），所有語音命令皆經由 `mpsc` 訊息佇列送往該執行緒執行，不在主執行緒或 WebView 執行緒直接呼叫 WinRT。
- **一次只唸一段**：新的 `speak_text` 會先停止前一段播放並遞增世代編號（`generation: AtomicU64`）；前一段晚到的 `CueEntered`／`MediaEnded`／`MediaFailed` 事件會因世代不符而被丟棄。
- **生命週期清理**：關閉分頁時前端停止朗讀；關閉視窗（`close_window`、`WindowEvent::Destroyed`）與 `SpeechService::drop` 時主行程也會停止播放並結束執行緒。

## 3. 只唸純文字與離線安全規則

1. **禁止 SSML**：合成只呼叫 `SpeechSynthesizer::SynthesizeTextToStreamAsync`，絕不呼叫 `SynthesizeSsmlToStreamAsync`（SSML 允許 `<audio src>` 與 `<lexicon uri>` 引用外部資源）。自動化測試 `winrt_speech_treats_ssml_and_sapi_markup_as_plain_text` 驗證 `<speak><audio src="http://example.invalid/a.wav"/></speak>` 與 `<voice required="Name=x"/>` 均被當成純文字朗讀。
2. **禁止 URI 與檔案路徑**：播放只使用 `MediaSource::CreateFromStream(&stream, &content_type)`，不使用 `CreateFromUri`，也不寫入暫存檔。
3. **語音 ID 白名單比對**：前端送來的 `voiceId` 只與 `SpeechSynthesizer::AllVoices()` 傳回的清單做精確字串比對，取得對應的 `VoiceInformation` 物件後傳給 `SetVoice`；不把 `voiceId` 當成檔案路徑或登錄機碼使用。
4. **輸入上限**：`crates/ipc_contract/src/limits.rs` 定義 `MAX_SPEECH_TEXT_BYTES`（512 KiB，涵蓋單頁 `MAX_PAGE_TEXT_CHARS` 100,000 字元上限）、`MIN_SPEECH_RATE`（0.5）與 `MAX_SPEECH_RATE`（6.0）、`MAX_SPEECH_VOICE_ID_BYTES`（512 bytes）。超過即拒絕。

## 4. UTF-16 與 Unicode 字元位置對照（`src/features/readAloud/model.ts`）

- WinRT `SpeechCue` 的 `StartPositionInInput` 與 `EndPositionInInput`（包含端點）以 **UTF-16 編碼單位**計算（已在 `winrt_speech_synthesizes_in_memory_with_utf16_word_boundaries` 以含非 BMP 字元 `U+20000` 的混合字串實測驗證：`utf16_len = end - start + 1`）。
- `PageText` 中 `TextLine.edges` 則以 **Unicode 純量值（code point）**索引（長度為 `Array.from(line.text).length + 1`）。非 BMP 字元佔 2 個 UTF-16 編碼單位，但在 `edges` 中只佔 1 個字元。
- 因此前端組出要唸的純文字時，同步建立對照表 `map: (Utf16Position | null)[]`，把每個 UTF-16 位置對應到 `{ page, line, charIndex }`（插入的換行 `\n` 對應 `null`）。收到 `SpeechEvent::Word { utf16Start, utf16Len }` 時，查表取出該詞涵蓋的字元範圍，並用 `TextLine.quad` 與 `TextLine.edges` 繪製不搶焦點的 SVG 標示（`aria-hidden`、`pointer-events-none`）。
- 播放中切換語音或速度時，以目前唸到的 UTF-16 位置切片剩餘文字並從該字重新朗讀，後續事件加上 `baseUtf16Offset` 維持標示對齊。

## 5. 不留紀錄與無障礙設計

- **不寫 log、不寫檔**：音訊與文字只在記憶體存活；`SpeakArgs` 的 `Debug` 實作刻意隱藏 `text` 內容；所有 `SpeechError` 都對應固定的靜態錯誤訊息，絕不含朗讀文字。
- **機敏文件**：機敏文件可正常朗讀，不寫入任何磁碟或歷程紀錄。
- **螢幕報讀軟體**：控制列只在狀態切換（開始、暫停、繼續、停止、完成、無文字、錯誤）時透過 `role="status"`（`aria-live="polite"`）通報一次，不在每個字更新時通報，避免干擾 NVDA 或 Windows 朗讀程式。
