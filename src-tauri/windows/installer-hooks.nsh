; NSIS hooks for the Tauri installer (bundle.windows.nsis.installerHooks in tauri.conf.json).

; pdf_worker runs in an AppContainer profile that crates/sandbox creates on first use
; (WORKER_APP_CONTAINER). The profile lives in the user's registry hive and
; %LOCALAPPDATA%\Packages, which the uninstaller does not otherwise touch, so remove it here.
; This covers the user running the uninstaller; profiles of other users on the same machine stay
; (see docs/architecture/worker-sandbox.md). A missing profile is not an error.
!macro NSIS_HOOK_POSTUNINSTALL
  System::Call 'userenv::DeleteAppContainerProfile(w "PdfReader.Worker") i .r0'
!macroend
