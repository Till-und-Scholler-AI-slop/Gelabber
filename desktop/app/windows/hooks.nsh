; Hooks into Tauri's NSIS installer (tauri.bundle.windows.json, installerHooks).

; The entry that names the app's toasts (src/notify/toast.rs `register`),
; which the app writes under its bundle identifier at every start. Gone with
; the app; an update starts the app again, which writes it again.
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\${BUNDLEID}"
!macroend
