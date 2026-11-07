; 22 Packaging (Windows): the installer, from packaging/windows/build.sh's
; stage folder. iscc /DVersion=… /DStage=… installer.iss

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId={{6A5E8D7C-3B0E-4C55-9C1E-6D2B8E4F0A11}
AppName=Oxrok UI
AppVersion={#Version}
DefaultDirName={autopf}\Oxrok UI
DefaultGroupName=Oxrok UI
OutputBaseFilename=Oxrok-UI-{#Version}-windows-x86_64-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
UninstallDisplayIcon={app}\bin\oxrok-ui.exe

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{group}\Oxrok UI"; Filename: "{app}\bin\oxrok-ui.exe"
Name: "{autodesktop}\Oxrok UI"; Filename: "{app}\bin\oxrok-ui.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Run]
Filename: "{app}\bin\oxrok-ui.exe"; Description: "Start Oxrok UI"; Flags: nowait postinstall skipifsilent
