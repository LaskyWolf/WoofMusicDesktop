; Inno Setup script for the Windows installer. CI runs:
;   iscc /DVersion=1.2.3 packaging\windows\woofmusic.iss
; after `cargo build --release` and libmpv.ps1 (which leaves the DLL in mpv\).
; Paths are relative to this file.

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId=com.laskywolf.woofmusic
AppName=WoofMusic
AppVersion={#Version}
AppPublisher=LaskyWolf
AppPublisherURL=https://github.com/LaskyWolf/WoofMusic
DefaultDirName={autopf}\WoofMusic
DisableProgramGroupPage=yes
; Installs for the current user without asking for admin rights.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\target\installer
OutputBaseFilename=WoofMusic-{#Version}-setup
SetupIconFile=woofmusic.ico
UninstallDisplayIcon={app}\woofmusic.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "..\..\target\release\woofmusic.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\mpv\libmpv*.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "woofmusic.ico"; DestDir: "{app}"

[Icons]
Name: "{autoprograms}\WoofMusic"; Filename: "{app}\woofmusic.exe"; IconFilename: "{app}\woofmusic.ico"
Name: "{autodesktop}\WoofMusic"; Filename: "{app}\woofmusic.exe"; IconFilename: "{app}\woofmusic.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\woofmusic.exe"; Description: "{cm:LaunchProgram,WoofMusic}"; Flags: nowait postinstall skipifsilent
