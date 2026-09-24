; NSIS hooks for the Tauri installer (bundle.windows.nsis.installerHooks in tauri.conf.json).
; Saved as UTF-8 with a BOM: NSIS reads a file without one in the system code page, which would
; garble the Chinese text below.

; The installer never downloads the WebView2 runtime (REL-02: webviewInstallMode "skip").
; Windows 11 includes it. If it is missing, say so before installing; the app explains it again
; when it starts (src-tauri/src/strings.rs).

; Sets $R0 to the installed WebView2 runtime's version, or to "" if there is none. The same
; registry check as Tauri's own WebView2 section.
!macro PDF_READER_WEBVIEW2_VERSION
  ${If} ${RunningX64}
    ReadRegStr $R0 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${Else}
    ReadRegStr $R0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  ${If} $R0 == ""
    ReadRegStr $R0 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  ${If} $R0 == "0.0.0.0"
    StrCpy $R0 ""
  ${EndIf}
!macroend

!define PDF_READER_WEBVIEW2_MISSING "這台電腦缺少 Microsoft Edge WebView2 Runtime，安裝完成後 PDF Reader 還無法開啟。$\r$\n$\r$\nWindows 11 已內建 WebView2。如果它被移除了，請到 Microsoft 官方網站下載並安裝「WebView2 Runtime」：$\r$\nhttps://developer.microsoft.com/microsoft-edge/webview2/$\r$\n$\r$\n本安裝程式不會自行下載任何東西。"

; /SD IDOK: a silent install does not stop here.
!macro NSIS_HOOK_PREINSTALL
  Push $R0
  !insertmacro PDF_READER_WEBVIEW2_VERSION
  ${If} $R0 == ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "${PDF_READER_WEBVIEW2_MISSING}" /SD IDOK
  ${EndIf}
  Pop $R0
!macroend

; pdf_worker runs in an AppContainer profile that crates/sandbox creates on first use
; (WORKER_APP_CONTAINER). The profile lives in the user's registry hive and
; %LOCALAPPDATA%\Packages, which the uninstaller does not otherwise touch, so remove it here.
; This covers the user running the uninstaller; profiles of other users on the same machine stay
; (see docs/architecture/worker-sandbox.md). A missing profile is not an error.
!macro NSIS_HOOK_POSTUNINSTALL
  System::Call 'userenv::DeleteAppContainerProfile(w "PdfReader.Worker") i .r0'
!macroend
