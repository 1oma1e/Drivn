; Drivn setup hooks (ADR-0028, 2026-11-01): the setup runs as administrator
; and turns the real mode on (R-TST-01 changed), so Drivn really installs on a friend's PC.
; The uninstaller removes it; journals and backups in ProgramData stay (rollback, R-SEC-03).

!macro NSIS_HOOK_POSTINSTALL
  SetShellVarContext all
  CreateDirectory "$COMMONAPPDATA\Drivn"
  FileOpen $0 "$COMMONAPPDATA\Drivn\real-mode" w
  FileWrite $0 "Created by the Drivn setup (ADR-0028)."
  FileClose $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  SetShellVarContext all
  Delete "$COMMONAPPDATA\Drivn\real-mode"
!macroend
