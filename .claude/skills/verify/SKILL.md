---
name: verify
description: 說「做完了」之前，自己在真正的 app 裡確認改動生效（verify）：建出 release 版的 app，用 repo 的 E2E harness 以 CDP 操作 WebView，或直接問沙盒中的 worker，再把證據寫進 PR。改到畫面、IPC 或 worker 的行為時，以及審查 PR 需要看 app 實際的樣子時使用。
---

# 在 app 裡驗證改動

做法只寫在一份文件：[docs/agents/verify.md](../../../docs/agents/verify.md)（從 repo 根目錄是 `docs/agents/verify.md`）。先讀完，再照著做；這個 skill 只是入口，不另外定規則。

動手之前先讀那份文件的「不能做」，例如：不開真正的外部連結、不截整個桌面、不讀寫剪貼簿、只開 `tests/corpus/` 的檔案、不操作桌面上的其他 app。完整的清單以文件為準。
