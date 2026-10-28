; Installer hooks.
;
; Up to 0.4.0 the program was `ai-usage.exe` with the identifier
; `cz.myspulin.aiusage`. The installer only knows how to stop a running
; `skylimit.exe`, and the single-instance lock follows the identifier, so an
; old copy left running would keep its own widget and tray icon next to the
; new one and hold the history database open. Stop it before installing.
;
; The history itself is never touched here: the app moves
; `%LOCALAPPDATA%\ai-usage-data` to `skylimit-data` on its first start.

!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /T /IM ai-usage.exe'
  Pop $0
  ; Let its WebView2 processes let go of their files.
  Sleep 800
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Tauri removes the old main binary itself (it remembers its name in the
  ; uninstall key); this covers an install whose key lost that value.
  Delete "$INSTDIR\ai-usage.exe"
  ; WebView2 cache stored under the old identifier. Only browser cache lives
  ; here; settings and history are in skylimit-data.
  RMDir /r "$LOCALAPPDATA\cz.myspulin.aiusage"
!macroend
