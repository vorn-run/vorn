; Vorn's Windows installer, built by package-native.mjs:
;   makensis /DVERSION= /DNUMERIC_VERSION= /DSOURCE= /DOUTFILE= /DICON= /DGUID= vorn.nsi
; It installs for the current user where the Electron app's installer did and
; takes over its registry entries, so an existing install updates in place.
; The updater runs it with /S --updated --force-run: silent, no folder
; choice, and Vorn started when done. A first install puts the folder with
; vorn.exe on the user's PATH; the uninstaller takes it off.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!include MUI2.nsh
!include FileFunc.nsh
!include LogicLib.nsh
!include WordFunc.nsh

!define APP "Vorn"
!define EXE "Vorn.exe"
!define UNINSTALLER "Uninstall Vorn.exe"
!define INSTALL_KEY "Software\${GUID}"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${GUID}"
!define URL_KEY "Software\Classes\vorn"
!define CLI_DIR "$INSTDIR\resources\vornd"

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\vorn"
InstallDirRegKey HKCU "${INSTALL_KEY}" InstallLocation
BrandingText "${APP} ${VERSION}"
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${NUMERIC_VERSION}"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${APP} Setup"
VIAddVersionKey "LegalCopyright" "MIT License"

Var forceRun
Var updated

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipOnUpdate
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipOnUpdate
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "--force-run" $1
  ${IfNot} ${Errors}
    StrCpy $forceRun 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $0 "--updated" $1
  ${IfNot} ${Errors}
    StrCpy $updated 1
  ${EndIf}
FunctionEnd

; An update keeps the folder and starts Vorn itself.
Function SkipOnUpdate
  ${If} $updated == 1
  ${OrIf} $forceRun == 1
    Abort
  ${EndIf}
FunctionEnd

Function .onInstSuccess
  ${If} $forceRun == 1
    Exec '"$INSTDIR\${EXE}"'
  ${EndIf}
FunctionEnd

; The app that started this update exits right after: wait for it, up to 10 seconds.
Function WaitForApp
  StrCpy $2 0
  ${DoWhile} ${FileExists} "$INSTDIR\${EXE}"
    ClearErrors
    FileOpen $3 "$INSTDIR\${EXE}" a
    ${IfNot} ${Errors}
      FileClose $3
      ${Break}
    ${EndIf}
    IntOp $2 $2 + 1
    ${If} $2 > 40
      ${Break}
    ${EndIf}
    Sleep 250
  ${Loop}
FunctionEnd

; A file still running (vornd, until the new app replaces it) cannot be
; overwritten, but can be renamed out of the way.
!macro MoveAside FILE
  ${If} ${FileExists} "$INSTDIR\${FILE}"
    Delete "$INSTDIR\${FILE}.old"
    ClearErrors
    Delete "$INSTDIR\${FILE}"
    ${If} ${Errors}
      Rename "$INSTDIR\${FILE}" "$INSTDIR\${FILE}.old"
      Delete /REBOOTOK "$INSTDIR\${FILE}.old"
    ${EndIf}
  ${EndIf}
!macroend

; What the Electron app installed that this one does not.
Function RemoveElectron
  ${If} ${FileExists} "$INSTDIR\resources\app.asar"
    Delete /REBOOTOK "$INSTDIR\*.dll"
    Delete /REBOOTOK "$INSTDIR\*.pak"
    Delete /REBOOTOK "$INSTDIR\*.bin"
    Delete /REBOOTOK "$INSTDIR\*.dat"
    Delete /REBOOTOK "$INSTDIR\*.json"
    Delete /REBOOTOK "$INSTDIR\LICENSE*"
    RMDir /r /REBOOTOK "$INSTDIR\locales"
    Delete /REBOOTOK "$INSTDIR\resources\app.asar"
    RMDir /r /REBOOTOK "$INSTDIR\resources\app.asar.unpacked"
    Delete "$INSTDIR\resources\app-update.yml"
    Delete "$INSTDIR\resources\elevate.exe"
  ${EndIf}
FunctionEnd

; Adds (+) or removes (-) the vorn command's folder in the user's PATH, for
; terminals opened after. A PATH too long to read whole is left alone: the
; app adds the folder itself on launch.
!macro UserPath OP
  ClearErrors
  ReadRegStr $0 HKCU "Environment" "Path"
  StrLen $1 $0
  IntOp $1 $1 + 260
  ${If} $1 < ${NSIS_MAX_STRLEN}
    ${If} $0 == ""
      StrCpy $1 ""
      ${If} "${OP}" == "+"
        StrCpy $1 "${CLI_DIR}"
      ${EndIf}
    ${Else}
      ${WordAdd} $0 ";" "${OP}${CLI_DIR}" $1
    ${EndIf}
    ${If} $1 != $0
      ${If} $1 == ""
        DeleteRegValue HKCU "Environment" "Path"
      ${Else}
        WriteRegExpandStr HKCU "Environment" "Path" $1
      ${EndIf}
      SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
    ${EndIf}
  ${EndIf}
!macroend

Section "Vorn"
  SetShellVarContext current
  Call WaitForApp
  !insertmacro MoveAside "${EXE}"
  !insertmacro MoveAside "resources\vornd\vornd.exe"
  !insertmacro MoveAside "resources\vornd\vorn.exe"
  !insertmacro MoveAside "resources\vornd\vorn-sessiond.exe"
  !insertmacro MoveAside "resources\vornd\conpty.dll"
  !insertmacro MoveAside "resources\vornd\x64\OpenConsole.exe"
  Call RemoveElectron
  ; The web client's file names change with every build.
  RMDir /r "$INSTDIR\resources\web"

  SetOutPath "$INSTDIR"
  File /r "${SOURCE}\*"
  WriteUninstaller "$INSTDIR\${UNINSTALLER}"

  CreateShortCut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}"
  ; On a first install, and kept up to date when the person kept it.
  ${IfNot} ${Silent}
  ${OrIf} ${FileExists} "$DESKTOP\${APP}.lnk"
    CreateShortCut "$DESKTOP\${APP}.lnk" "$INSTDIR\${EXE}"
  ${EndIf}

  WriteRegStr HKCU "${INSTALL_KEY}" InstallLocation "$INSTDIR"
  WriteRegStr HKCU "${INSTALL_KEY}" KeepShortcuts "true"
  WriteRegStr HKCU "${INSTALL_KEY}" ShortcutName "${APP}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" DisplayName "${APP} ${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" DisplayVersion "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" DisplayIcon "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" Publisher "${APP}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" InstallLocation "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" UninstallString '"$INSTDIR\${UNINSTALLER}" /currentuser'
  WriteRegStr HKCU "${UNINSTALL_KEY}" QuietUninstallString '"$INSTDIR\${UNINSTALLER}" /currentuser /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" NoModify 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" NoRepair 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" EstimatedSize "$0"

  WriteRegStr HKCU "${URL_KEY}" "" "URL:Vorn"
  WriteRegStr HKCU "${URL_KEY}" "URL Protocol" ""
  WriteRegStr HKCU "${URL_KEY}\DefaultIcon" "" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "${URL_KEY}\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'

  ; An update keeps whatever the person chose in Settings.
  ${If} $updated != 1
    !insertmacro UserPath "+"
  ${EndIf}
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  !insertmacro UserPath "-"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  RMDir /r /REBOOTOK "$INSTDIR\resources"
  Delete /REBOOTOK "$INSTDIR\${EXE}"
  Delete /REBOOTOK "$INSTDIR\*.old"
  Delete "$INSTDIR\${UNINSTALLER}"
  RMDir /REBOOTOK "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "${INSTALL_KEY}"
  DeleteRegKey HKCU "${URL_KEY}"
SectionEnd
