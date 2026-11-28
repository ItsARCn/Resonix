; Used by test-msi.py to exercise Inno Setup -> WiX MSI upgrade takeover.
; Releases after 0.14.2 ship only the Windows Installer package (resonix.wxs).
;
;   iscc /DVersion=0.17.2 /DArch=x86_64 /DBinary=...\resonix.exe ^
;        /DOutputDir=dist packaging\windows\resonix.iss

#ifndef Version
  #error Version must be defined on the ISCC command line
#endif
#ifndef Arch
  #error Arch must be defined on the ISCC command line (x86_64 or aarch64)
#endif
#ifndef Binary
  #error Binary must be defined on the ISCC command line
#endif
#ifndef OutputDir
  #error OutputDir must be defined on the ISCC command line
#endif
#if Arch == "aarch64"
  #define InnoArch "arm64"
#else
  #define InnoArch "x64compatible"
#endif

#define AppName "Resonix"
#define AppExeName "resonix.exe"
#define AppIdentity "Resonix"

[Setup]
; Never change: this is how Windows tells an update from a new program.
AppId={{05B3C8C9-9B3A-492B-BD6E-44F4850B3377}
AppName={#AppName}
AppVersion={#Version}
AppVerName={#AppName} {#Version}
AppPublisher=ItsARCn
AppCopyright=© 2026 ItsARCn
AppPublisherURL=https://github.com/ItsARCn/Resonix
AppSupportURL=https://github.com/ItsARCn/Resonix/issues
AppUpdatesURL=https://github.com/ItsARCn/Resonix/releases
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed={#InnoArch}
ArchitecturesInstallIn64BitMode={#InnoArch}
MinVersion=10.0
LicenseFile=..\..\LICENSE
OutputDir={#OutputDir}
OutputBaseFilename=resonix-v{#Version}-{#Arch}-pc-windows-msvc-setup
SetupIconFile=resonix.ico
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\{#AppExeName}
; The file version has to be numbers: a release candidate's -rc1 comes off.
#define Dash Pos("-", Version)
#if Dash > 0
  #define NumericVersion Copy(Version, 1, Dash - 1)
#else
  #define NumericVersion Version
#endif
VersionInfoVersion={#NumericVersion}.0

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#Binary}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "resonix-installer.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Classes\resonix"; ValueType: string; ValueName: ""; ValueData: "URL:Resonix link"
Root: HKCU; Subkey: "Software\Classes\resonix"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\resonix\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"",0"
Root: HKCU; Subkey: "Software\Classes\resonix\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Resonix.resonix"; ValueType: string; ValueName: ""; ValueData: "URL:Resonix link"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Resonix.resonix"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Resonix.resonix\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"",0"
Root: HKCU; Subkey: "Software\Classes\Resonix.resonix\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKCU; Subkey: "Software\{#AppIdentity}\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\{#AppIdentity}\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "A native YouTube Music client"
Root: HKCU; Subkey: "Software\{#AppIdentity}\Capabilities\URLAssociations"; ValueType: string; ValueName: "resonix"; ValueData: "Resonix.resonix"
Root: HKCU; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#AppIdentity}"; ValueData: "Software\{#AppIdentity}\Capabilities"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[Code]
#if Arch == "x86_64"
// x64compatible lets this installer run on an ARM PC under emulation, and a
// copy installed from it keeps updating to x64. The ARM installer has
// everything this one has and runs natively, so point to it. A silent
// install (an update) goes ahead without asking.
function InitializeSetup(): Boolean;
begin
  Result := True;
  if IsArm64 and not WizardSilent then
    Result := MsgBox('This PC has an ARM processor. The ARM installer ' +
      '(aarch64) runs natively, faster and on less battery.' + #13#10#13#10 +
      'Install the version for Intel and AMD processors anyway?',
      mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;
#endif

procedure CloseInstalledResonix;
var
  Locator, Services, Processes, Process, Path: Variant;
  Index: Integer;
  Exe: String;
begin
  Exe := ExpandConstant('{app}\{#AppExeName}');
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Services := Locator.ConnectServer('', 'root\CIMV2');
    Processes := Services.ExecQuery('SELECT * FROM Win32_Process WHERE Name = ''{#AppExeName}''');
    for Index := 0 to Processes.Count - 1 do begin
      Process := Processes.ItemIndex(Index);
      Path := Process.ExecutablePath;
      // A portable copy can have the same name. Never close it by name alone.
      if not VarIsNull(Path) then
        if CompareText(Path, Exe) = 0 then
          Log('Closing installed Resonix: result ' + IntToStr(Process.Terminate(0)));
    end;
  except
    Log('Could not close installed Resonix: ' + GetExceptionMessage);
  end;
end;

// CloseApplications covers Setup, not Uninstall. Check the installed file
// before removing anything, including the uninstaller needed to try again.
function InitializeUninstall: Boolean;
var
  Exe: String;
  Probe: TFileStream;
begin
  Result := False;
  Exe := ExpandConstant('{app}\{#AppExeName}');
  while FileExists(Exe) do begin
    try
      // Windows denies write access to a running executable. Do not write
      // anything: opening it also catches other locks on this exact copy.
      Probe := TFileStream.Create(Exe, fmOpenReadWrite or fmShareExclusive);
      Probe.Free;
      Result := True;
      Exit;
    except
      Log('Cannot open the installed executable for uninstall: ' + GetExceptionMessage);
    end;
    if UninstallSilent then
      Exit;
    case SuppressibleTaskDialogMsgBox('Resonix cannot be removed yet.',
      'Its program file is in use or inaccessible. Quit Resonix and retry, or close it here to continue.' + #13#10#13#10 +
      'Close Resonix ends the process immediately. Unsaved changes may be lost.',
      mbError, MB_YESNOCANCEL, ['Retry', 'Close Resonix'], 0, IDCANCEL) of
      IDYES: ;
      IDNO: begin
        CloseInstalledResonix;
        Sleep(200);
      end;
    else
      Exit;
    end;
  end;
  Result := True;
end;

// The resonix: scheme key is shared with whatever else opens the links, so
// uninstalling takes it away only while it still names this program.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command: String;
  Exe: String;
begin
  if CurUninstallStep <> usUninstall then
    Exit;
  if not RegQueryStringValue(HKCU, 'Software\Classes\resonix\shell\open\command', '', Command) then
    Exit;
  Exe := Lowercase(ExpandConstant('{app}\{#AppExeName}'));
  if Pos(Exe, Lowercase(Command)) > 0 then
    RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Classes\resonix');
end;
