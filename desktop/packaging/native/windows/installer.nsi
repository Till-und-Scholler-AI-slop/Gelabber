; Windows installer of the native app (desktop/gelabber), made with makensis
; in desktop-native.yml. It takes the Tauri app's place as its update: the
; same folder, uninstall entry, shortcuts and AppUserModelId as Tauri's NSIS
; installer (desktop/app/tauri.bundle.windows.json, windows/hooks.nsh), and
; the arguments its updater passes (/P /UPDATE /R /ARGS).
;
;   makensis /DVERSION=<x.y.z[-pre]> /DFILEVERSION=<x.y.z.0> /DSOURCE=<dir> /DOUTFILE=<setup.exe> installer.nsi
;
; <dir> holds gelabber.exe, gelabber_media.dll, icon.ico, icon.png,
; README.txt and THIRD-PARTY-NOTICES.txt.
; Arguments of the setup: /S silent, /P progress only, /UPDATE keeps the
; shortcuts as they are, /R starts the app afterwards with what follows
; /ARGS, /D=<folder> (last) installs elsewhere.

!macro NEEDS name
  !ifndef ${name}
    !error "/D${name}=... is needed (see the head of installer.nsi)"
  !endif
!macroend
!insertmacro NEEDS VERSION
!insertmacro NEEDS FILEVERSION
!insertmacro NEEDS SOURCE
!insertmacro NEEDS OUTFILE

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
; Into the account's own folders, as the Tauri app (installMode currentUser).
RequestExecutionLevel user

!define PRODUCTNAME "Gelabber"
!define MANUFACTURER "Gelabber"
; desktop/app/tauri.conf.json identifier: the toasts' and shortcuts' AppUserModelId.
!define BUNDLEID "io.github.till-und-scholler-ai-slop.gelabber"
!define MAINBINARY "gelabber.exe"
!define TAURIBINARY "gelabber-desktop.exe"
!define HOMEPAGE "https://github.com/Till-und-Scholler-AI-slop/Gelabber"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
; Where Tauri's installer notes the folder it installed to.
!define MANUPRODUCTKEY "Software\${MANUFACTURER}\${PRODUCTNAME}"
!define AUMIDKEY "Software\Classes\AppUserModelId\${BUNDLEID}"

Name "${PRODUCTNAME}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\${PRODUCTNAME}"
InstallDirRegKey HKCU "${MANUPRODUCTKEY}" ""
BrandingText "${PRODUCTNAME} ${VERSION}"
Icon "${SOURCE}\icon.ico"
UninstallIcon "${SOURCE}\icon.ico"

VIProductVersion "${FILEVERSION}"
VIFileVersion "${FILEVERSION}"
VIAddVersionKey /LANG=0 "ProductName" "${PRODUCTNAME}"
VIAddVersionKey /LANG=0 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "FileDescription" "${PRODUCTNAME} Setup"
VIAddVersionKey /LANG=0 "CompanyName" "${MANUFACTURER}"
VIAddVersionKey /LANG=0 "LegalCopyright" "${MANUFACTURER}"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"

Var PassiveMode
Var UpdateMode
Var Restart
Var AppArgs

!define MUI_ICON "${SOURCE}\icon.ico"
!define MUI_UNICON "${SOURCE}\icon.ico"
!define MUI_ABORTWARNING

!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenPassive
!insertmacro MUI_PAGE_WELCOME
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenPassive
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenPassive
!define MUI_FINISHPAGE_RUN "$INSTDIR\${MAINBINARY}"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; Tauri's languages (tauri.bundle.windows.json); the system's is chosen.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "German"

LangString appRunning ${LANG_ENGLISH} "Gelabber is running. Close it and continue?"
LangString appRunning ${LANG_GERMAN} "Gelabber läuft noch. Beenden und fortfahren?"
LangString needs64 ${LANG_ENGLISH} "Gelabber needs a 64-bit Windows (x64)."
LangString needs64 ${LANG_GERMAN} "Gelabber braucht ein 64-Bit-Windows (x64)."
LangString deleteData ${LANG_ENGLISH} "Also delete Gelabber's settings and sign-in on this computer?"
LangString deleteData ${LANG_GERMAN} "Auch die Einstellungen und die Anmeldung von Gelabber auf diesem Computer löschen?"

; Ends both apps of this account (this one, or the Tauri app it replaces):
; their files are about to be replaced. Asks first unless the setup runs
; without questions (/S, /P, /UPDATE).
!macro CLOSE_APPS un
  Function ${un}CloseApps
    Push $0
    Push $1
    !insertmacro CLOSE_ONE ${MAINBINARY}
    !insertmacro CLOSE_ONE ${TAURIBINARY}
    Pop $1
    Pop $0
  FunctionEnd
!macroend

!macro CLOSE_ONE binary
  ; find.exe ends with 0 when tasklist names the program.
  nsExec::ExecToStack `"$SYSDIR\cmd.exe" /c ""$SYSDIR\tasklist.exe" /NH /FI "IMAGENAME eq ${binary}" /FI "USERNAME eq %USERNAME%" | "$SYSDIR\find.exe" /I "${binary}" >NUL"`
  Pop $0
  Pop $1
  ${If} $0 = 0
    ${If} $PassiveMode <> 1
    ${AndIf} $UpdateMode <> 1
    ${AndIfNot} ${Silent}
      MessageBox MB_OKCANCEL|MB_ICONINFORMATION "$(appRunning)" IDOK +2
      Quit
    ${EndIf}
    nsExec::Exec `"$SYSDIR\taskkill.exe" /F /T /IM ${binary} /FI "USERNAME eq %USERNAME%"`
    Pop $0
    Sleep 1500
  ${EndIf}
!macroend

!insertmacro CLOSE_APPS ""
!insertmacro CLOSE_APPS "un."

; Sets System.AppUserModel.ID on a shortcut, as Tauri's installer does, so
; that Windows groups the window and the toasts under the app's id. Plain
; COM through the System plugin: IShellLinkW, IPersistFile, IPropertyStore.
!macro SET_LNK_AUMID shortcut
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $9
  StrCpy $0 0
  System::Call 'ole32::CoCreateInstance(g "{00021401-0000-0000-C000-000000000046}", p 0, i 1, g "{000214F9-0000-0000-C000-000000000046}", *p .r0) i .r9'
  ${If} $9 = 0
    StrCpy $1 0
    System::Call '$0->0(g "{0000010b-0000-0000-C000-000000000046}", *p .r1) i .r9'
    ${If} $9 = 0
      ; IPersistFile::Load with STGM_READWRITE.
      System::Call '$1->5(w "${shortcut}", i 2) i .r9'
      StrCpy $2 0
      System::Call '$0->0(g "{886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99}", *p .r2) i .r9'
      ${If} $9 = 0
        ; PROPERTYKEY PKEY_AppUserModel_ID; PROPVARIANT VT_LPWSTR (31) with
        ; the id in a buffer of its own that lives until the commit.
        System::Call '*(&g16 "{9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}", i 5) p .r3'
        System::Call '*(&w256 "${BUNDLEID}") p .r5'
        System::Call '*(&i2 31, &i2 0, &i2 0, &i2 0, p r5, p 0) p .r4'
        System::Call '$2->6(p r3, p r4) i .r9'
        System::Call '$2->7() i .r9'
        System::Call '$2->2()'
        System::Free $3
        System::Free $4
        System::Free $5
        ; IPersistFile::Save to the loaded file.
        System::Call '$1->6(p 0, i 1) i .r9'
      ${EndIf}
      System::Call '$1->2()'
    ${EndIf}
    System::Call '$0->2()'
  ${EndIf}
  Pop $9
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

!macro SHORTCUT lnk
  CreateShortcut "${lnk}" "$INSTDIR\${MAINBINARY}" "" "$INSTDIR\icon.ico" 0
  !insertmacro SET_LNK_AUMID "${lnk}"
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "$(needs64)" /SD IDOK
    Abort
  ${EndIf}
  SetRegView 64
  StrCpy $PassiveMode 0
  StrCpy $UpdateMode 0
  StrCpy $Restart 0
  StrCpy $AppArgs ""
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/P" $R1
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/R" $R1
  ${IfNot} ${Errors}
    StrCpy $Restart 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/ARGS" $R1
  ${IfNot} ${Errors}
    StrCpy $AppArgs $R1
  ${EndIf}
  ${If} $PassiveMode = 1
    SetAutoClose true
  ${EndIf}
FunctionEnd

Function SkipWhenPassive
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    Abort
  ${EndIf}
FunctionEnd

Section "Install"
  SetShellVarContext current
  Call CloseApps
  SetOutPath "$INSTDIR"
  File "${SOURCE}\${MAINBINARY}"
  File "${SOURCE}\gelabber_media.dll"
  ; icon.png names the toasts (ui/src/platform/notify/toast.rs).
  File "${SOURCE}\icon.png"
  File "${SOURCE}\icon.ico"
  File "${SOURCE}\README.txt"
  File "${SOURCE}\THIRD-PARTY-NOTICES.txt"
  ; The Tauri app this one replaces.
  Delete "$INSTDIR\${TAURIBINARY}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "${MANUPRODUCTKEY}" "" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  ; The binary has no icon resource of its own.
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$\"$INSTDIR\icon.ico$\""
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  WriteRegStr HKCU "${UNINSTKEY}" "URLInfoAbout" "${HOMEPAGE}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$\"$INSTDIR$\""
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" "$0"

  ; An update keeps the shortcuts the user has, pointed at the new binary
  ; (the Tauri app's led to gelabber-desktop.exe).
  ${If} $UpdateMode = 1
    ${If} ${FileExists} "$SMPROGRAMS\${PRODUCTNAME}.lnk"
      !insertmacro SHORTCUT "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ${EndIf}
    ${If} ${FileExists} "$DESKTOP\${PRODUCTNAME}.lnk"
      !insertmacro SHORTCUT "$DESKTOP\${PRODUCTNAME}.lnk"
    ${EndIf}
  ${Else}
    !insertmacro SHORTCUT "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    !insertmacro SHORTCUT "$DESKTOP\${PRODUCTNAME}.lnk"
  ${EndIf}
SectionEnd

Function .onInstSuccess
  ${If} $Restart = 1
    Exec '"$INSTDIR\${MAINBINARY}" $AppArgs'
  ${EndIf}
FunctionEnd

Function un.onInit
  SetRegView 64
  StrCpy $PassiveMode 0
  StrCpy $UpdateMode 0
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  Call un.CloseApps
  Delete "$INSTDIR\${MAINBINARY}"
  Delete "$INSTDIR\${TAURIBINARY}"
  Delete "$INSTDIR\gelabber_media.dll"
  Delete "$INSTDIR\icon.png"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\uninstall.exe"
  ; Only when empty: the folder may have been chosen with /D.
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  ; Written by the app at every start; gone with the app (hooks.nsh).
  DeleteRegKey HKCU "${AUMIDKEY}"

  ; Settings and sign-in stay unless the user asks, never in a silent run.
  MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "$(deleteData)" /SD IDNO IDNO keep
  RMDir /r "$APPDATA\${BUNDLEID}"
  RMDir /r "$LOCALAPPDATA\${BUNDLEID}"
  DeleteRegKey HKCU "${MANUPRODUCTKEY}"
  keep:
SectionEnd
