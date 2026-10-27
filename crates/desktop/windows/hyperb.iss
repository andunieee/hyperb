; Built by the windows workflow: iscc /DVersion=0.1.0 /DBundle=<bundle dir> /DOutput=<dir> hyperb.iss
[Setup]
AppId={{6B0C3F5E-2B7A-4F0D-9C1E-5D7A1E4B8A21}
AppName=hyperb
AppVersion={#Version}
AppPublisher=andunie
DefaultDirName={autopf}\hyperb
DefaultGroupName=hyperb
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#Output}
OutputBaseFilename=hyperb-windows-x64-setup-v{#Version}
SetupIconFile=hyperb.ico
UninstallDisplayIcon={app}\bin\hyperb-desktop.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes

[Files]
Source: "{#Bundle}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{group}\hyperb"; Filename: "{app}\bin\hyperb-desktop.exe"
Name: "{autodesktop}\hyperb"; Filename: "{app}\bin\hyperb-desktop.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

; hyperb:// and web+hyperb:// links open in the app; --links refuses local files
[Registry]
Root: HKA; Subkey: "Software\Classes\hyperb"; ValueType: string; ValueData: "URL:hyperb"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\hyperb"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\hyperb\DefaultIcon"; ValueType: string; ValueData: "{app}\bin\hyperb-desktop.exe,0"
Root: HKA; Subkey: "Software\Classes\hyperb\shell\open\command"; ValueType: string; ValueData: """{app}\bin\hyperb-desktop.exe"" --links ""%1"""
Root: HKA; Subkey: "Software\Classes\web+hyperb"; ValueType: string; ValueData: "URL:web+hyperb"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\web+hyperb"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\web+hyperb\DefaultIcon"; ValueType: string; ValueData: "{app}\bin\hyperb-desktop.exe,0"
Root: HKA; Subkey: "Software\Classes\web+hyperb\shell\open\command"; ValueType: string; ValueData: """{app}\bin\hyperb-desktop.exe"" --links ""%1"""

[Run]
Filename: "{app}\bin\hyperb-desktop.exe"; Description: "Launch hyperb"; Flags: nowait postinstall skipifsilent
