; First Aid Cat — Windows setup (Inno Setup 6.3 or newer). The file is UTF-8 with a BOM on purpose: Inno reads Cyrillic only then.
;
;   ISCC.exe /DAppVersion=0.9.1 /DSourceExe=C:\path\aptechka.exe packaging\windows\first-aid-cat.iss
;
;   AppVersion  (required) version from engine/Cargo.toml
;   SourceExe   the cross-built aptechka.exe (default: engine\target\x86_64-pc-windows-gnu\release\aptechka.exe)
; Result: dist\first-aid-cat-VERSION-windows-x86_64-setup.exe
;
; Per-user install without UAC, into %LOCALAPPDATA%\Programs\First Aid Cat. Start menu shortcuts are named by the
; installer language (English: First Aid Cat, Russian: Аптечка). Settings in %APPDATA%\aptechka are left on uninstall.

#ifndef AppVersion
  #error Pass the version: ISCC.exe /DAppVersion=0.9.1 ...
#endif
#ifndef SourceExe
  #define SourceExe "..\..\engine\target\x86_64-pc-windows-gnu\release\aptechka.exe"
#endif
; the documents: repository root (public repo) or release\ (development repo)
#if FileExists(AddBackslash(SourcePath) + "..\..\LICENSE")
  #define DocDir "..\.."
#else
  #define DocDir "..\..\release"
#endif

[Setup]
; the GUID identifies the app for upgrades and the uninstall entry — never change it
AppId={{6F0C2B7E-3A41-4D8B-9E5A-7C1D4B2A8F36}
AppName=First Aid Cat
AppVersion={#AppVersion}
AppPublisher=UlyanaPerova
AppPublisherURL=https://github.com/UlyanaPerova/first-aid-cat
AppSupportURL=https://github.com/UlyanaPerova/first-aid-cat/issues
AppUpdatesURL=https://github.com/UlyanaPerova/first-aid-cat/releases
VersionInfoVersion={#AppVersion}.0
VersionInfoProductName=First Aid Cat
VersionInfoDescription=First Aid Cat setup
; per-user install, no UAC: {autopf} then means %LOCALAPPDATA%\Programs
PrivilegesRequired=lowest
DefaultDirName={autopf}\First Aid Cat
DefaultGroupName={cm:AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SetupIconFile=..\..\assets\aptechka.ico
UninstallDisplayIcon={app}\aptechka.exe
LicenseFile={#DocDir}\LICENSE
OutputDir=..\..\dist
OutputBaseFilename=first-aid-cat-{#AppVersion}-windows-x86_64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; close a running copy that holds the files being replaced (only processes using files from {app}) ...
CloseApplications=yes
CloseApplicationsFilter=aptechka.exe
RestartApplications=no
; ... and the code below stops the copy from {app} before install and uninstall

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
english.AppName=First Aid Cat
english.WidgetName=First Aid Cat — widget
english.TaskOptions=Additional options:
english.TaskWidgetAutostart=Start the widget at sign-in
english.TaskDesktopIcon=Create a desktop shortcut
english.LaunchApp=Start First Aid Cat
russian.AppName=Аптечка
russian.WidgetName=Аптечка — виджет
russian.TaskOptions=Дополнительно:
russian.TaskWidgetAutostart=Запускать виджет при входе в систему
russian.TaskDesktopIcon=Создать ярлык на рабочем столе
russian.LaunchApp=Запустить Аптечку

[Tasks]
; checked by default
Name: "widgetautostart"; Description: "{cm:TaskWidgetAutostart}"; GroupDescription: "{cm:TaskOptions}"
Name: "desktopicon"; Description: "{cm:TaskDesktopIcon}"; GroupDescription: "{cm:TaskOptions}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "aptechka.exe"; Flags: ignoreversion
Source: "{#DocDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#DocDir}\NOTICE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#DocDir}\LICENSE-ART.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#DocDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#DocDir}\PRIVACY.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{cm:AppName}"; Filename: "{app}\aptechka.exe"
Name: "{group}\{cm:WidgetName}"; Filename: "{app}\aptechka.exe"; Parameters: "--widget"
; a shortcut in the Startup folder (the same name as the one made by install.ps1, so no second copy appears)
Name: "{userstartup}\{cm:WidgetName}"; Filename: "{app}\aptechka.exe"; Parameters: "--widget"; Tasks: widgetautostart
Name: "{autodesktop}\{cm:AppName}"; Filename: "{app}\aptechka.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\aptechka.exe"; Description: "{cm:LaunchApp}"; Flags: nowait postinstall skipifsilent

[Code]
// Stops aptechka.exe started from {app} — and only that one: other programs and a copy
// from another folder (for example a portable one) are not touched.
procedure StopOurCopy();
var
  Exe, Args: String;
  Code: Integer;
begin
  Exe := ExpandConstant('{app}\aptechka.exe');
  StringChangeEx(Exe, '''', '''''', True);   // ' inside a PowerShell string is written ''
  Args := '-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Get-Process aptechka -ErrorAction SilentlyContinue ' +
          '| Where-Object { $_.Path -eq ''' + Exe + ''' } | ForEach-Object { $_.Kill(); [void]$_.WaitForExit(5000) }"';
  Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'), Args, '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopOurCopy();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    StopOurCopy();
end;
