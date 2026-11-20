; ============================================================================
;  QookiX Launcher NSIS installer/uninstaller hooks
;  These macros are invoked by the custom installer.nsi via !insertmacro:
;    NSIS_HOOK_PREINSTALL / NSIS_HOOK_POSTINSTALL / NSIS_HOOK_PREUNINSTALL
; ============================================================================
Var /GLOBAL QXOldInstallDir

; ----------------------------------------------------------------------------
;  PREINSTALL: close any running instance so files aren't locked during an
;  upgrade, and remember the previous install location to repair shortcuts.
; ----------------------------------------------------------------------------
!macro NSIS_HOOK_PREINSTALL
  ; Only terminate a running instance when there IS one to overwrite.
  ; taskkill enumerates every process (including lsass.exe) and force-kills,
  ; which scores as process enumeration / HIPS heuristics on a first-time
  ; install where no previous binary exists. Skip it for fresh installs.
  ${If} $UpdateMode = 1
  ${OrIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    ExecWait 'taskkill /im "${MAINBINARYNAME}.exe" /f' $0
  ${EndIf}
  ; Remember previous install location (stored quoted in the registry) so the
  ; post-install step can fix shortcuts that still point to the old path.
  ReadRegStr $QXOldInstallDir SHCTX "${UNINSTKEY}" "InstallLocation"
  StrCpy $QXOldInstallDir $QXOldInstallDir "" 1
  StrCpy $QXOldInstallDir $QXOldInstallDir -1 ""
!macroend

; ----------------------------------------------------------------------------
;  POSTINSTALL: if the desktop shortcut still points at the previous install
;  directory, redirect it to the new one (happens on upgrades).
; ----------------------------------------------------------------------------
!macro NSIS_HOOK_POSTINSTALL
  !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$QXOldInstallDir\${MAINBINARYNAME}.exe"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  ${EndIf}
  ; --- qookix:// protocol registration (runs on every install AND upgrade) ---
  ; The template's deep_link_protocols loop is not reliably executed on
  ; silent in-app upgrades (observed: 0.5.4 -> 0.5.5 lost the association),
  ; so register it explicitly here as belt-and-braces. Idempotent: each run
  ; overwrites with identical values, so upgrades stay correct after the
  ; install dir changes.
  WriteRegStr SHCTX "Software\Classes\qookix" "URL Protocol" ""
  WriteRegStr SHCTX "Software\Classes\qookix" "" "URL:qookix protocol"
  WriteRegStr SHCTX "Software\Classes\qookix\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr SHCTX "Software\Classes\qookix\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
!macroend

; ----------------------------------------------------------------------------
;  POSTUNINSTALL: remove the persistent installer UI binary written by .onInit.
;  Only our own directory is touched, and only for a real uninstall (updates
;  re-use this copy on the next install).
; ----------------------------------------------------------------------------
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    Delete "$LOCALAPPDATA\${PRODUCTNAME}\InstallerUI\QookiXInstallerUI.exe"
    RMDir "$LOCALAPPDATA\${PRODUCTNAME}\InstallerUI"
    RMDir "$LOCALAPPDATA\${PRODUCTNAME}"
  ${EndIf}
!macroend
