; Inno Setup script for the Relay Windows installer.
; Built by .github/workflows/windows.yml:
;   iscc /DAppVersion=0.1.0 /DSourceExe=..\server\target\release\relay.exe installer\relay.iss
;
; What it does:
;  - installs relay.exe to Program Files\Relay
;  - Start menu: "Relay" (starts the server), "Pair phone" (shows the QR code),
;    "Relay data folder"; optional desktop icon and start-with-Windows
;  - adds a firewall rule that lets only Tailscale addresses (100.64.0.0/10)
;    reach port 8765; nothing is opened to the internet or the local network
;  - points you to Tailscale if it isn't installed
;  - starts Relay at the end. On first start Relay finds your GPU, downloads the
;    matching llama.cpp build, picks a model from LM Studio and shows the QR code.
;
; Your settings, chat history, logs and downloaded llama.cpp live in
; %LOCALAPPDATA%\Relay and are kept on uninstall unless you choose otherwise.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\server\target\release\relay.exe"
#endif

#define AppName "Relay"
#define FirewallRule "Relay (Tailscale only)"

[Setup]
AppId={{6C1F3B2E-4E5A-4C38-9B1E-2E7D9C5A1F40}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Relay
AppComments=Run local coding agents on this PC and control them from your phone over Tailscale.
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=Output
OutputBaseFilename=RelaySetup-{#AppVersion}
SetupIconFile=relay.ico
UninstallDisplayIcon={app}\relay.ico
UninstallDisplayName={#AppName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
WelcomeLabel2=This will install [name/ver] on your computer.%n%nRelay runs coding agents on this PC with your own GPU (llama.cpp) and lets you chat with them from the Relay phone app, anywhere, over Tailscale.%n%nEverything is set up automatically on first start.

[Tasks]
Name: "startup"; Description: "Start Relay when I sign in to Windows"; GroupDescription: "Options:"
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Options:"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "relay.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\server\config.example.toml"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion

[Icons]
Name: "{group}\Relay"; Filename: "{app}\relay.exe"; Parameters: "serve"; WorkingDir: "{app}"; IconFilename: "{app}\relay.ico"; Comment: "Start the Relay server (keep the window open)"
Name: "{group}\Pair phone"; Filename: "{app}\relay.exe"; Parameters: "pair"; WorkingDir: "{app}"; IconFilename: "{app}\relay.ico"; Comment: "Show the QR code for the Relay phone app"
Name: "{group}\Relay data folder"; Filename: "{localappdata}\Relay"; Comment: "Settings, logs, models and chat history"
Name: "{group}\Uninstall Relay"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Relay"; Filename: "{app}\relay.exe"; Parameters: "serve"; WorkingDir: "{app}"; IconFilename: "{app}\relay.ico"; Tasks: desktopicon
Name: "{userstartup}\Relay"; Filename: "{app}\relay.exe"; Parameters: "serve"; WorkingDir: "{app}"; IconFilename: "{app}\relay.ico"; Flags: runminimized; Tasks: startup

[Run]
; Allow inbound connections to Relay only from Tailscale's address range.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""{#FirewallRule}"""; Flags: runhidden waituntilterminated; StatusMsg: "Configuring Windows Firewall..."
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""{#FirewallRule}"" dir=in action=allow protocol=TCP localport=8765 remoteip=100.64.0.0/10 program=""{app}\relay.exe"" profile=any enable=yes"; Flags: runhidden waituntilterminated; StatusMsg: "Configuring Windows Firewall..."
Filename: "https://tailscale.com/download/windows"; Description: "Get Tailscale (needed to reach Relay from your phone)"; Flags: postinstall shellexec skipifsilent runasoriginaluser; Check: not TailscaleInstalled
Filename: "{app}\relay.exe"; Parameters: "serve"; WorkingDir: "{app}"; Description: "Start Relay now and pair your phone"; Flags: postinstall nowait skipifsilent runasoriginaluser

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/IM relay.exe /T /F"; Flags: runhidden; RunOnceId: "StopRelay"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""{#FirewallRule}"""; Flags: runhidden; RunOnceId: "RemoveFirewallRule"

[Code]
function TailscaleInstalled: Boolean;
begin
  Result := FileExists(ExpandConstant('{commonpf64}\Tailscale\tailscale.exe'))
    or FileExists(ExpandConstant('{commonpf32}\Tailscale\tailscale.exe'))
    or RegKeyExists(HKLM, 'SOFTWARE\Tailscale IPN');
end;

{ Stop a running Relay (and its llama-server) so the exe can be replaced. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM relay.exe /T /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  { Silent installs skip this; SuppressibleMsgBox also honours /SUPPRESSMSGBOXES. }
  if (CurPageID = wpReady) and not WizardSilent and not TailscaleInstalled then
    SuppressibleMsgBox('Tailscale was not found on this PC.' + #13#10#13#10 +
      'Relay only accepts connections over Tailscale, so install Tailscale on this PC and on your phone ' +
      'and sign in with the same account. You can do that after this setup finishes.',
      mbInformation, MB_OK, IDOK);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    DataDir := ExpandConstant('{localappdata}\Relay');
    if DirExists(DataDir) and not UninstallSilent then
      if SuppressibleMsgBox('Also delete your Relay data?' + #13#10#13#10 + DataDir + #13#10#13#10 +
          'This holds your settings, chat history, logs, the downloaded llama.cpp and anything you put in its models folder. ' +
          'Your project folders are never touched.',
          mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
        DelTree(DataDir, True, True, True);
  end;
end;
