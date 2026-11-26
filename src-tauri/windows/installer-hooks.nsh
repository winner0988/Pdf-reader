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

; Two-language strings (ADR 0029, #685). Tauri includes installerHooks before MUI_LANGUAGE (so
; ${LANG_ENGLISH}/${LANG_TRADCHINESE} are not defined yet), and NSIS forbids LangString inside a
; Section (so they cannot go inside NSIS_HOOK_PREINSTALL / NSIS_HOOK_POSTINSTALL). Numeric Windows
; LCIDs (1033 = English, 1028 = TradChinese, matching tauri.conf.json order) work at top level.
LangString PDF_READER_WEBVIEW2_MISSING 1033 "This computer is missing Microsoft Edge WebView2 Runtime, so PDF Reader cannot open after installation.$\r$\n$\r$\nWindows 11 includes WebView2. If it has been removed, please download and install WebView2 Runtime from the official Microsoft website:$\r$\nhttps://developer.microsoft.com/microsoft-edge/webview2/$\r$\n$\r$\nThis installer does not download anything on its own."
LangString PDF_READER_WEBVIEW2_MISSING 1028 "這台電腦缺少 Microsoft Edge WebView2 Runtime，安裝完成後 PDF Reader 還無法開啟。$\r$\n$\r$\nWindows 11 已內建 WebView2。如果它被移除了，請到 Microsoft 官方網站下載並安裝「WebView2 Runtime」：$\r$\nhttps://developer.microsoft.com/microsoft-edge/webview2/$\r$\n$\r$\n本安裝程式不會自行下載任何東西。"

LangString PDF_READER_CREATE_PDF_LABEL 1033 "Create PDF with PDF Reader"
LangString PDF_READER_CREATE_PDF_LABEL 1028 "用 PDF Reader 建立 PDF"

LangString PDF_READER_PROGID_DESCRIPTION 1033 "PDF Document"
LangString PDF_READER_PROGID_DESCRIPTION 1028 "PDF 文件"

LangString PDF_READER_APP_DESCRIPTION 1033 "Privacy-first, offline PDF reader"
LangString PDF_READER_APP_DESCRIPTION 1028 "注重隱私、完全離線的 PDF 閱讀器"

; /SD IDOK: a silent install does not stop here.
!macro NSIS_HOOK_PREINSTALL
  Push $R0
  !insertmacro PDF_READER_WEBVIEW2_VERSION
  ${If} $R0 == ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "$(PDF_READER_WEBVIEW2_MISSING)" /SD IDOK
  ${EndIf}
  Pop $R0
!macroend

; PDF Reader as a PDF app (REL-03): a ProgID for .pdf, "Open with", and the Default apps page of
; Windows Settings (Capabilities + RegisteredApplications). The .pdf default itself is never
; changed: Windows 10 and 11 let only the user choose it, and the app's "set as default" opens
; the Settings page for that. The 64-bit registry view: RegisteredApplications is not shared
; with the 32-bit view that the installer uses by default.
!define PDF_READER_PROGID "PdfReader.Document"
!define PDF_READER_CAPABILITIES "Software\PDF Reader\Capabilities"

; Explorer context menu verb for images (B5-13, #450): registered under SystemFileAssociations so
; image ProgIDs, OpenWithProgids, UserChoice, and default handlers are never modified.
; MultiSelectModel=Player raises Windows Explorer's static-verb selection limit from 15 (Document)
; to 100 items; Windows invokes the command once per selected file and single-instance collects them.
!define PDF_READER_CREATE_PDF_VERB "PdfReader.CreatePdf"

!macro PDF_READER_REGISTER_IMAGE_VERB ext
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}" "MUIVerb" "$(PDF_READER_CREATE_PDF_LABEL)"
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}" "Icon" "$INSTDIR\${MAINBINARYNAME}.exe,0"
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}" "MultiSelectModel" "Player"
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe" --create-pdf-from "%1"'
!macroend

!macro PDF_READER_UNREGISTER_IMAGE_VERB ext
  DeleteRegKey HKLM "Software\Classes\SystemFileAssociations\.${ext}\shell\${PDF_READER_CREATE_PDF_VERB}"
!macroend

!macro PDF_READER_REGISTER
  SetRegView 64
  WriteRegStr HKLM "Software\Classes\${PDF_READER_PROGID}" "" "$(PDF_READER_PROGID_DESCRIPTION)"
  ; The PDF file icon, resource 32514 of the exe (UX-06, src-tauri/build.rs): a page, not the app icon.
  WriteRegStr HKLM "Software\Classes\${PDF_READER_PROGID}\DefaultIcon" "" "$INSTDIR\${MAINBINARYNAME}.exe,-32514"
  WriteRegStr HKLM "Software\Classes\${PDF_READER_PROGID}\shell\open\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe" "%1"'
  WriteRegStr HKLM "Software\Classes\.pdf\OpenWithProgids" "${PDF_READER_PROGID}" ""
  WriteRegStr HKLM "Software\Classes\Applications\${MAINBINARYNAME}.exe" "FriendlyAppName" "${PRODUCTNAME}"
  WriteRegStr HKLM "Software\Classes\Applications\${MAINBINARYNAME}.exe\SupportedTypes" ".pdf" ""
  WriteRegStr HKLM "Software\Classes\Applications\${MAINBINARYNAME}.exe\shell\open\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe" "%1"'
  WriteRegStr HKLM "${PDF_READER_CAPABILITIES}" "ApplicationName" "${PRODUCTNAME}"
  WriteRegStr HKLM "${PDF_READER_CAPABILITIES}" "ApplicationDescription" "$(PDF_READER_APP_DESCRIPTION)"
  WriteRegStr HKLM "${PDF_READER_CAPABILITIES}" "ApplicationIcon" "$INSTDIR\${MAINBINARYNAME}.exe,0"
  WriteRegStr HKLM "${PDF_READER_CAPABILITIES}\FileAssociations" ".pdf" "${PDF_READER_PROGID}"
  WriteRegStr HKLM "Software\RegisteredApplications" "${PRODUCTNAME}" "${PDF_READER_CAPABILITIES}"
  !insertmacro PDF_READER_REGISTER_IMAGE_VERB "png"
  !insertmacro PDF_READER_REGISTER_IMAGE_VERB "jpg"
  !insertmacro PDF_READER_REGISTER_IMAGE_VERB "jpeg"
  !insertmacro PDF_READER_REGISTER_IMAGE_VERB "gif"
  !insertmacro PDF_READER_REGISTER_IMAGE_VERB "bmp"
  !insertmacro PDF_READER_REGISTER_IMAGE_VERB "tif"
  !insertmacro PDF_READER_REGISTER_IMAGE_VERB "tiff"
  SetRegView lastused
  ; SHCNE_ASSOCCHANGED, SHCNF_FLUSH: Explorer picks up the new handler without a restart.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x1000, p 0, p 0)'
!macroend

!macro PDF_READER_UNREGISTER
  SetRegView 64
  !insertmacro PDF_READER_UNREGISTER_IMAGE_VERB "png"
  !insertmacro PDF_READER_UNREGISTER_IMAGE_VERB "jpg"
  !insertmacro PDF_READER_UNREGISTER_IMAGE_VERB "jpeg"
  !insertmacro PDF_READER_UNREGISTER_IMAGE_VERB "gif"
  !insertmacro PDF_READER_UNREGISTER_IMAGE_VERB "bmp"
  !insertmacro PDF_READER_UNREGISTER_IMAGE_VERB "tif"
  !insertmacro PDF_READER_UNREGISTER_IMAGE_VERB "tiff"
  DeleteRegValue HKLM "Software\RegisteredApplications" "${PRODUCTNAME}"
  DeleteRegKey HKLM "Software\PDF Reader"
  DeleteRegKey HKLM "Software\Classes\Applications\${MAINBINARYNAME}.exe"
  DeleteRegValue HKLM "Software\Classes\.pdf\OpenWithProgids" "${PDF_READER_PROGID}"
  DeleteRegKey HKLM "Software\Classes\${PDF_READER_PROGID}"
  SetRegView lastused
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x1000, p 0, p 0)'
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro PDF_READER_REGISTER
!macroend

; pdf_worker runs in an AppContainer profile that crates/sandbox creates on first use
; (WORKER_APP_CONTAINER). The profile lives in the user's registry hive and
; %LOCALAPPDATA%\Packages, which the uninstaller does not otherwise touch, so remove it here.
; This covers the user running the uninstaller; profiles of other users on the same machine stay
; (see docs/architecture/worker-sandbox.md). A missing profile is not an error.
!macro NSIS_HOOK_POSTUNINSTALL
  System::Call 'userenv::DeleteAppContainerProfile(w "PdfReader.Worker") i .r0'
  !insertmacro PDF_READER_UNREGISTER
!macroend
