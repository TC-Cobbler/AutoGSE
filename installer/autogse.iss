#define MyAppName "AutoGSE"
#define MyAppVersion "0.2.0"
#define MyAppPublisher "AutoGSE Project"
#define MyAppExeName "autogse.exe"

; Repo root is one level up from this script.
#define RepoRoot "..\"

[Setup]
AppId={{BDD72098-6E2A-48C7-9539-8DEC14FC937F}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\AutoGSE
DefaultGroupName=AutoGSE
DisableProgramGroupPage=yes
; Installs to Program Files and the vendored tools tree is large — both
; require standard admin elevation, unrelated to AutoGSE's own per-user
; (HKCU-only) context-menu/AUMID registration, which needs no elevation.
PrivilegesRequired=admin
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#RepoRoot}dist
OutputBaseFilename=AutoGSE-Setup-{#MyAppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
InfoBeforeFile=ATTRIBUTION.txt
SetupIconFile={#RepoRoot}assets\app.ico
UninstallDisplayIcon={app}\{#MyAppExeName}

[Files]
Source: "{#RepoRoot}target\release\autogse.exe"; DestDir: "{app}"; Flags: ignoreversion
; Phase 11 §11.1: the IExplorerCommand COM shell extension DLL. Must land in
; the same directory as autogse.exe — registry.rs's install_context_menu
; resolves this DLL's path as a sibling of autogse.exe's own current_exe(),
; not via a separate configurable location.
Source: "{#RepoRoot}target\release\autogse_shell.dll"; DestDir: "{app}"; Flags: ignoreversion
; Vendored alex47exe/gse_fork tooling. The destination folder name
; ("gen_emu_cfg") is not arbitrary — it's exactly what
; goldberg::tools_root()'s release-mode branch expects beside the exe, per
; Phase 3's design. Do not rename this folder without also updating that
; function.
Source: "{#RepoRoot}alex47exe-gse_fork\gen_emu_cfg-Windows-Release\generate_emu_config\*"; DestDir: "{app}\gen_emu_cfg"; Flags: ignoreversion recursesubdirs createallsubdirs
; Phase 6 additions: these three tools are siblings of generate_emu_config/
; in the vendored source tree, not inside it, so they need their own
; destination folders matching goldberg.rs's matching resolver functions
; (parse_controller_vdf_root/lobby_connect_root/steamclient_experimental_root).
Source: "{#RepoRoot}alex47exe-gse_fork\gen_emu_cfg-Windows-Release\parse_controller_vdf\*"; DestDir: "{app}\parse_controller_vdf"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#RepoRoot}alex47exe-gse_fork\release\tools\lobby_connect\*"; DestDir: "{app}\lobby_connect"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#RepoRoot}alex47exe-gse_fork\release\steamclient_experimental\*"; DestDir: "{app}\steamclient_experimental"; Flags: ignoreversion recursesubdirs createallsubdirs
; Phase 10 §10.3: Steamless (SteamStub DRM unpacker) lives at the repo root,
; not under alex47exe-gse_fork/ — it's a separate, CC BY-NC-ND-licensed tool
; unrelated to gse_fork's own tooling. Destination folder name must match
; goldberg::steamless_root()'s release-mode branch.
Source: "{#RepoRoot}Steamless\*"; DestDir: "{app}\Steamless"; Flags: ignoreversion recursesubdirs createallsubdirs

[Run]
Filename: "{app}\{#MyAppExeName}"; Parameters: "install-menu"; Flags: runhidden waituntilterminated; StatusMsg: "Registering Explorer context menu..."

[UninstallRun]
; Runs before [Files] are removed, so the registry cleanup (and Start Menu
; shortcut removal) it performs still has a valid exe path to work with.
Filename: "{app}\{#MyAppExeName}"; Parameters: "uninstall-menu"; Flags: runhidden waituntilterminated; RunOnceId: "AutoGseUninstallMenu"
