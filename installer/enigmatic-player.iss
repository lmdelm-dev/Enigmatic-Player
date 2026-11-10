; Enigmatic Player - Windows installer
;
; Builds a single setup.exe that installs the player and, while installing,
; fetches everything the player needs at runtime:
;
;   * Microsoft Visual C++ 2015-2022 runtime (the exe imports VCRUNTIME140.dll)
;   * ffmpeg  (decode online streams)         ) only when the "online tools"
;   * yt-dlp  (resolve provider stream urls)  ) task is left enabled
; Compile with:  installer\build.ps1   (or ISCC.exe installer\enigmatic-player.iss)

#define AppName      "Enigmatic Player"
#define AppVersion   "1.8.0"
#define AppExe       "enigmatic-player.exe"
#define AppPublisher "Jamal Tiliouan"
#define AppUrl       "https://github.com/lmdelm-dev/Enigmatic-Player"

#define VcRedistUrl  "https://aka.ms/vs/17/release/vc_redist.x64.exe"
#define YtDlpUrl     "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
#define FfmpegUrl    "https://github.com/eugeneware/ffmpeg-static/releases/latest/download/ffmpeg-win32-x64"

[Setup]
AppId={{7C1B9B7E-5E4C-4E2E-9B6F-1B7D2B4B9E11}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputDir=dist
OutputBaseFilename=enigmatic-player-{#AppVersion}-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=commandline
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName} {#AppVersion}
LicenseFile=..\LICENSE
VersionInfoVersion={#AppVersion}
VersionInfoDescription={#AppName} setup
VersionInfoProductName={#AppName}
VersionInfoProductVersion={#AppVersion}
VersionInfoCopyright=Copyright (c) 2026 {#AppPublisher}

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"
Name: "online"; Description: "Download the online streaming tools (yt-dlp + ffmpeg, about 100 MB)"; GroupDescription: "Online sources:"

[Files]
Source: "..\enigmatic-player-rust\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{tmp}\vc_redist.x64.exe"; Parameters: "/install /quiet /norestart"; StatusMsg: "Installing the Microsoft Visual C++ runtime..."; Flags: waituntilterminated runhidden; Check: VcRedistDownloaded
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: files; Name: "{app}\ffmpeg.exe"
Type: files; Name: "{app}\yt-dlp.exe"
Type: dirifempty; Name: "{app}"

[Code]
const
  VcRedistUrl  = '{#VcRedistUrl}';
  YtDlpUrl     = '{#YtDlpUrl}';
  FfmpegUrl    = '{#FfmpegUrl}';

var
  DownloadPage: TDownloadWizardPage;
  NeedVc, WantOnline: Boolean;

{ True when the VC++ 2015-2022 x64 runtime is already registered. }
function VcRuntimeInstalled: Boolean;
var
  Installed: Cardinal;
begin
  Result :=
    (RegQueryDWordValue(HKLM64, 'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64',
                        'Installed', Installed) and (Installed = 1)) or
    (RegQueryDWordValue(HKLM, 'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64',
                        'Installed', Installed) and (Installed = 1));
end;

function VcRedistDownloaded: Boolean;
begin
  Result := NeedVc and FileExists(ExpandConstant('{tmp}\vc_redist.x64.exe'));
end;

function FfmpegDownloaded: Boolean;
begin
  Result := WantOnline and FileExists(ExpandConstant('{tmp}\ffmpeg.exe'));
end;

function YtDlpDownloaded: Boolean;
begin
  Result := WantOnline and FileExists(ExpandConstant('{tmp}\yt-dlp.exe'));
end;

{ The tools were downloaded into the setup temp folder; place them next to
  the player so the player's child processes find them in its own directory
  (Windows resolves "ffmpeg"/"yt-dlp" from the calling executable's folder first). }
procedure CurStepChanged(CurStep: TSetupStep);
var
  Dst: String;
begin
  if CurStep <> ssPostInstall then
    Exit;
  Dst := ExpandConstant('{app}\ffmpeg.exe');
  if FfmpegDownloaded and not FileCopy(ExpandConstant('{tmp}\ffmpeg.exe'), Dst, False) then
    MsgBox('ffmpeg could not be placed in ' + ExtractFileDir(Dst) + '.' + #13#10 +
           'Online streaming will not work until you copy ffmpeg.exe there yourself.',
           mbError, MB_OK);
  Dst := ExpandConstant('{app}\yt-dlp.exe');
  if YtDlpDownloaded and not FileCopy(ExpandConstant('{tmp}\yt-dlp.exe'), Dst, False) then
    MsgBox('yt-dlp could not be placed in ' + ExtractFileDir(Dst) + '.' + #13#10 +
           'Online streaming will not work until you copy yt-dlp.exe there yourself.',
           mbError, MB_OK);
end;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage(
    'Downloading the player dependencies',
    'The player needs a few components that are not shipped inside this setup. ' +
    'They are downloaded now, once, and stay on this computer.',
    nil);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Items: Integer;
begin
  Result := '';
  NeedVc := not VcRuntimeInstalled;
  WantOnline := WizardIsTaskSelected('online');

  Items := 0;
  DownloadPage.Clear;
  if NeedVc then
  begin
    DownloadPage.Add(VcRedistUrl, 'vc_redist.x64.exe', '');
    Inc(Items);
  end;
  if WantOnline then
  begin
    DownloadPage.Add(YtDlpUrl, 'yt-dlp.exe', '');
    DownloadPage.Add(FfmpegUrl, 'ffmpeg.exe', '');
    Items := Items + 2;
  end;
  if Items = 0 then
    Exit;

  { Silent installs cannot show the page, but the download still has to run. }
  if not WizardSilent then
    DownloadPage.Show;
  try
    try
      DownloadPage.Download;
    except
      Result := 'Could not download the dependencies. Check the connection ' +
                'and run the setup again, or install them manually: ' + #13#10 +
                '  VC++ runtime: ' + VcRedistUrl + #13#10 +
                '  yt-dlp: ' + YtDlpUrl + #13#10 +
                '  ffmpeg: ' + FfmpegUrl;
    end;
  finally
    if not WizardSilent then
      DownloadPage.Hide;
  end;
end;
