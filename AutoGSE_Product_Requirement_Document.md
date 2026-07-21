# Product Requirement Document (PRD)
# AutoGSE: Automated Goldberg Achievement & Emulator Integrator

**Document Status:** Draft / Revision 2.0  
**Author:** Lead Systems Architect & Product Engineering  
**Target Release:** v1.0.0-STABLE  
**Target Platform:** Windows 10 / 11 (x64) & Wine/Proton Layer  

---

## 1. Document Overview & Executive Summary

### 1.1 Executive Summary
**AutoGSE** is a zero-GUI, high-performance command-line utility and shell extension designed to streamline the integration of offline Steam achievements for non-Steam games and alternative launchers (e.g., Hydra Launcher, Heroic, custom setups). It automates the configuration, injection, and maintenance of the **Goldberg Steam Emulator (`gse_fork` by alex47exe)** and synchronizes live achievement metadata with local tracking frameworks like **Achievement Watcher**.

By leveraging context menu integration, recursive PE binary inspection, fuzzy metadata scraping, and automated wrapper execution, AutoGSE eliminates the complex, error-prone manual steps traditionally required to set up offline achievement tracking.

### 1.2 Purpose & Scope
This document outlines the end-to-end technical requirements, functional specifications, data structures, non-functional requirements, edge-case protocols, and implementation plan for building the AutoGSE toolchain.

---

## 2. Problem Statement & High-Level Objectives

### 2.1 The Problem
1. **Complex Binary Hierarchy:** Modern game engines (e.g., Unreal Engine 4/5, Unity, Frostbite, REDengine) place executable launchers in the root directory (e.g., `Game.exe`), while the actual Steamworks API wrappers (`steam_api.dll` or `steam_api64.dll`) reside deep within nested subdirectories (e.g., `[Root]/Engine/Binaries/Win64/` or `[Root]/Binaries/Retail/`). Blindly placing emulator files next to the root `.exe` causes silent failures or game crashes.
2. **Tedious Manual Setup:** Injecting Goldberg's emulator manually requires users to:
   - Identify whether the binary is 32-bit (`x86`) or 64-bit (`x64`).
   - Locate the official Steam App ID via web browsers or SteamDB.
   - Run external configuration tools (`generate_emu_config.exe`, `gse_generate_interfaces.exe`, `gse_acw_helper.exe`).
   - Manually populate `steam_appid.txt`, `steam_settings/`, and achievement JSON schemas.
   - Rename original files to maintain backup copies.
3. **Lack of Native Rollback:** Gamers attempting to restore vanilla structures or apply official game patches often break their game installs due to leftover emulator DLLs, lost original files, or corrupted configurations.

### 2.2 Core Product Objectives
* **Single-Click Execution:** Inject or revert achievement emulators directly from the Windows Context Menu on any game folder or executable.
* **Intelligent File Targeting:** Automatically resolve the exact directory hosting `steam_api.dll` or `steam_api64.dll` regardless of nested depth.
* **Automated Identification:** Deduce the Steam App ID with >95% accuracy using executable PE metadata, folder string sanitization, local checks, and remote API queries.
* **Full Integration Ecosystem:** Produce 100% compatible file structures for Goldberg (`gse_fork`), Achievement Watcher, and Hydra Launcher without requiring manual file edits.
* **Deterministic Rollback:** Guarantee 100% file restoration via an atomic tracking manifest (`.gse_manifest.json`).

---

## 3. Product Vision & User Personas

### 3.1 Target Personas

#### Persona A: "The Casual Offline Gamer" (Alex, 24)
* **Goal:** Wants games downloaded from third-party sources or DRM-free stores to record achievements just like Steam.
* **Pain Point:** Doesn't understand DLL architectures, directory trees, or Steam App IDs. Easily frustrated by multi-step manual tutorials.
* **Needs:** A "right-click -> setup achievements" button that "just works."

#### Persona B: "The Achievement Completionist" (Elena, 29)
* **Goal:** Tracks every unlocked trophy across all games inside Achievement Watcher or Hydra Launcher.
* **Pain Point:** Frequently finds games where achievements unlock in-game but fail to communicate with Achievement Watcher because `gse_acw_helper.exe` or `achievements.json` was generated incorrectly.
* **Needs:** Flawless schema generation and real-time notification hooks.

#### Persona C: "The Modder / Tinkerer" (Marcus, 32)
* **Goal:** Tests custom mods, offline patches, and game builds across various handheld devices (Steam Deck / Windows handhelds).
* **Pain Point:** Broken emulators that leave modified files scattered without a clean uninstall path.
* **Needs:** Clean rollback mechanisms, non-destructive file backups, and low system overhead.

---

## 4. Architectural Overview & System Components

### 4.1 Modular Component Architecture

```
                                  +---------------------------------------+
                                  |     Windows Shell (Context Menu)      |
                                  +-------------------+-------------------+
                                                      |
                                                      v
                                  +-------------------+-------------------+
                                  |            AutoGSE Core CLI           |
                                  +-------------------+-------------------+
                                                      |
           +------------------------------------------+------------------------------------------+
           |                                          |                                          |
           v                                          v                                          v
+----------+----------+                    +----------+----------+                    +----------+----------+
|  Directory & PE     |                    |   App ID Discovery   |                    |   GSE Toolkit Wrapper   |
|  Scanner Module     |                    |        Engine        |                    |        Module       |
+----------+----------+                    +----------+----------+                    +----------+----------+
| - Recursive search   |                    | - Local manifest check   |                    | - gse_fork_tools    |
| - Bitness check     |                    | - PE metadata extraction |                    | - Interface gen     |
| - Target directory  |                    | - Sanitized fuzzy string|                    | - ACW schema gen    |
|   pinpointing       |                    | - Steam API / SteamDB    |                    | - Payload build     |
+----------+----------+                    +----------+----------+                    +----------+----------+
           |                                          |                                          |
           +------------------------------------------+------------------------------------------+
                                                      |
                                                      v
                                  +-------------------+-------------------+
                                  |     Backup & Mutex State Engine       |
                                  +-------------------+-------------------+
                                  | - .gse_manifest.json logging          |
                                  | - Atomic rename / copy operations     |
                                  | - Revert / Rollback executor          |
                                  +---------------------------------------+
```

---

## 5. Functional Requirements (Deep Dive)

### 5.1 Windows Shell Integration & Context Menu Engine (FR-1)

#### 5.1.1 Registry Configuration
AutoGSE registers background shell entries for both files (`.exe`) and directories (`Folder`).

* **Registry Keys (Files):**
  - `HKEY_CLASSES_ROOT\exe\shell\AutoGSE_Inject`
  - `HKEY_CLASSES_ROOT\exe\shell\AutoGSE_Revert`
* **Registry Keys (Directories):**
  - `HKEY_CLASSES_ROOT\Directory\shell\AutoGSE_Inject`
  - `HKEY_CLASSES_ROOT\Directory\shell\AutoGSE_Revert`
* **Command Strings:**
  ```cmd
  "C:\Program Files\AutoGSE\autogse.exe" inject --path "%1"
  "C:\Program Files\AutoGSE\autogse.exe" revert --path "%1"
  ```

#### 5.1.2 Dynamic Menu Visibility
- If a valid `.gse_manifest.json` exists in the targeted folder hierarchy, the context menu dynamically prioritizes the **Revert / Rollback** action.
- If no manifest exists, **Inject Achievement Emulator** is highlighted.

---

### 5.2 Recursive Binary & Target Directory Discovery Engine (FR-2)

#### 5.2.1 Search Algorithm & Rules
When executed on a target path P:
1. If P is a file (`.exe`), set D_root = Directory(P).
2. If P is a directory, set D_root = P.
3. Execute a breadth-first search (BFS) up to a max depth of **6 directory levels** beneath D_root.
4. Scan for target filenames:
   - `steam_api.dll` (32-bit / x86)
   - `steam_api64.dll` (64-bit / x64)

#### 5.2.2 Engine Path Pinpointing Examples

| Game Engine | Context Selected Path | Resolved Target Operating Directory (TOD) | Target DLL |
| :--- | :--- | :--- | :--- |
| **Unreal Engine 4/5** | `[Root]/Cyberpunk.exe` | `[Root]/Engine/Binaries/Win64/` | `steam_api64.dll` |
| **Unity Engine** | `[Root]/HollowKnight.exe` | `[Root]/` | `steam_api.dll` |
| **RE Engine** | `[Root]/RE2.exe` | `[Root]/` | `steam_api64.dll` |
| **Nested Custom** | `[Root]/Launcher.exe` | `[Root]/bin/x64/` | `steam_api64.dll` |

#### 5.2.3 Architecture Verification (Bitness Inspection)
Before copying Goldberg emulator DLLs, AutoGSE reads the PE header of the target `steam_api(64).dll`:
- Read `IMAGE_DOS_HEADER.e_lfanew` -> `IMAGE_NT_HEADERS.FileHeader.Machine`.
- `0x014c` => **IMAGE_FILE_MACHINE_I386 (32-bit)** => Deploy 32-bit Goldberg DLL.
- `0x8664` => **IMAGE_FILE_MACHINE_AMD64 (64-bit)** => Deploy 64-bit Goldberg DLL.

---

### 5.3 Multi-Tiered Steam App ID Discovery Engine (FR-3)

To ensure automatic operation without user entry, AutoGSE resolves the Steam App ID using a cascading 5-step heuristic pipeline:

```
[Target Path / Executable]
           |
           v
+------------------------------------+
| Step 1: Check Local Manifest/TXT   | ---> Found? ---> [Return App ID]
+------------------------------------+
           | No
           v
+------------------------------------+
| Step 2: Parse PE Version Resources | ---> Found App ID String? ---> [Return App ID]
+------------------------------------+
           | No
           v
+------------------------------------+
| Step 3: Sanitize Folder/File Name  |
+------------------------------------+
           |
           v
+------------------------------------+
| Step 4: Query Steam Store API      | ---> Match > 85% Confidence? ---> [Return App ID]
+------------------------------------+
           | No
           v
+------------------------------------+
| Step 5: Interactive Terminal UI    | ---> User Selects / Manual Input ---> [Return App ID]
+------------------------------------+
```

#### 5.3.1 Local Search Checks (Step 1)
Scan TOD and parent folders for existing files:
- `steam_appid.txt` -> Read integer content.
- `steam_interfaces.txt` or legacy emulator configs.

#### 5.3.2 PE Version Resource Scraper (Step 2)
Parse PE metadata fields (`FileDescription`, `ProductName`, `Comments`, `OriginalFilename`) using binary inspection. Frequently, games contain strings like:
`Assembly Version: 1.0.0.0 | SteamAppID: 1091500` or `Product Name: Cyberpunk 2077`.

#### 5.3.3 Sanitization & Fuzzy Matcher (Step 3 & 4)
Sanitize folder names using regular expressions to strip noise words:
* **Regex Filters:** `\b(v\d+\.\d+|FitGirl|DODI|Repack|Deluxe|Edition|GOG|MULTi\d+|crack|FLT|TENOKE|RUNE|Goldberg)\b`
* **Cleaned String:** `Cyberpunk 2077 v1.63 MULTi12-FitGirl` --> `Cyberpunk 2077`
* **Steam Web API Query:** `GET https://api.steampowered.com/ISteamApps/GetAppList/v2/`
* Calculate string similarity against official Steam app list using **Jaro-Winkler distance metric** or **Levenshtein Distance** (Similarity Threshold >= 0.88).

#### 5.3.4 Fallback Interactive Prompt (Step 5)
If API queries fail or return low confidence, pop up a lightweight console modal:
```
===================================================================
 AutoGSE - Steam App ID Disambiguation
===================================================================
 Target Directory: C:\Games\UnknownGame\
 Could not auto-verify Steam App ID with high confidence.

 Top Candidate Matches:
 [1] Unknown Game: Modern Warfare (AppID: 123456) - Confidence: 72%
 [2] Unknown Game Edition (AppID: 654321) - Confidence: 68%
 [3] Enter Custom Steam App ID manually

 Select an option [1-3]: _
===================================================================
```

---

### 5.4 Goldberg Orchestration & Payload Packaging (FR-4)

AutoGSE bundles and wraps `alex47exe/gse_fork_tools` dependencies natively.

#### 5.4.1 Tool Invocation Sequence
1. **Config & Schema Generation:**
   Execute `generate_emu_config.exe_anon` in headless mode passing flags:
   ```bash
   generate_emu_config.exe_anon.exe --appid <APP_ID> --acw --output-dir "C:\AppData\Local\Temp\AutoGSE_<APP_ID>"
   ```
   *The `--acw` flag triggers automated packaging of Achievement Watcher compatible database schemas (`achievements.json`, image asset folders).*

2. **Interface Export Engine:**
   Run `gse_generate_interfaces.exe` targeting the original `steam_api(64).dll` to export the exact C++ interface function pointers (`steam_interfaces.txt`).

3. **Achievement Watcher Helper Integration:**
   Deploy `gse_acw_helper.exe` into the TOD to bridge live achievement unlocks with `%APPDATA%\Achievement Watcher\`.

---

### 5.5 Atomic File Injection & Backup Engine (FR-5 & FR-6)

To prevent file corruption, half-written configurations, or broken game installs, AutoGSE uses an atomic file execution pipeline.

#### 5.5.1 Injection Workflow Lifecycle

```
[Start Injection]
       |
       v
Check file write permissions & process locks
       |
       +---> [If Process Running] ---> Abort with Error Toast
       |
       v
Is original steam_api.dll backed up?
       |
       +--- No ---> RENAME "steam_api.dll" -> "steam_api.dll.org"
       |            RENAME "steam_api64.dll" -> "steam_api64.dll.org"
       |
       v
COPY Goldberg Emulator DLL -> "steam_api.dll" / "steam_api64.dll"
       |
       v
WRITE "steam_appid.txt" with <APP_ID>
       |
       v
INJECT "steam_settings/" folder containing:
  ├── force_language.txt (default: english)
  ├── user_steam_id.txt (76561198000000000)
  ├── achievements.json
  └── steam_interfaces.txt
       |
       v
GENERATE & WRITE ".gse_manifest.json" in TOD
       |
       v
[Complete - Display Success Toast]
```

#### 5.5.2 Atomic State Manifest Schema (`.gse_manifest.json`)
Every modified folder contains a hidden manifest file tracking mutated state:

```json
{
  "version": "1.0.0",
  "timestamp": "2026-07-20T10:15:00Z",
  "app_id": 1091500,
  "game_title": "Cyberpunk 2077",
  "target_directory": "C:\\Games\\Cyberpunk 2077\\bin\\x64",
  "arch": "x64",
  "backed_up_files": [
    {
      "original_path": "steam_api64.dll",
      "backup_path": "steam_api64.dll.org",
      "sha256_hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    }
  ],
  "injected_files": [
    "steam_api64.dll",
    "steam_appid.txt",
    "gse_acw_helper.exe",
    "steam_settings/achievements.json",
    "steam_settings/steam_interfaces.txt",
    "steam_settings/force_language.txt",
    "steam_settings/user_steam_id.txt"
  ]
}
```

#### 5.5.3 De-Injection & Rollback Routine (FR-6)
When the user clicks **Rollback to Vanilla Structure**:
1. Read `.gse_manifest.json` in TOD.
2. Verify hashes of `.org` backup files.
3. Delete all items listed in `injected_files`.
4. Rename `steam_api(64).dll.org` back to `steam_api(64).dll`.
5. Remove `steam_settings/` folder recursively.
6. Delete `.gse_manifest.json`.
7. Pop up notification: `"Successfully restored vanilla game state for [Game Name]."`.

---

## 6. Achievement Watcher & Hydra Launcher Compatibility

### 6.1 Data Synchronization Pathways
To guarantee that unlocked achievements trigger instant OS popups and log into Achievement Watcher / Hydra Launcher databases:

1. **Local File Hooks:** `achievements.json` must strictly adhere to the schema required by Goldberg's notification module:
   ```json
   [
     {
       "name": "NEW_ACHIEVEMENT_1",
       "default_value": 0,
       "displayName": "Welcome to Night City",
       "hidden": 0,
       "description": "Arrive in Night City.",
       "icon": "achievements/NEW_ACHIEVEMENT_1_off.jpg",
       "icongray": "achievements/NEW_ACHIEVEMENT_1_on.jpg"
     }
   ]
   ```
2. **Directory Syncing:** Copy achievement icon assets into:
   - `TOD/steam_settings/achievements/`
   - `%APPDATA%/Goldberg SteamEmu Saves/settings/`
3. **Watcher Process Signal:** Trigger a local pipe or UDP broadcast to notify Achievement Watcher to scan the newly generated App ID manifest.

---

## 7. Technical Specifications & Non-Functional Requirements

### 7.1 Performance Metrics
* **Directory Scan Duration:** < 300 ms for directory depth of 6 levels.
* **App ID Fuzzy Resolution Time:** < 1.2 s (with active network connection).
* **Total Execution Time:** Single context menu click to active injection completed in < 2.5 s.
* **Executable Footprint:** Compiled self-contained single binary < 15 MB.

### 7.2 Safety, Elevation & Permissions
* **UAC Handling:** AutoGSE manifests include `requestedExecutionLevel level="asInvoker"`. If write access to `C:\Program Files\` or restricted folders fails with `EACCES`, re-launch CLI context using Windows `ShellExecute` verb `runas` to prompt for UAC elevation.
* **Anti-Virus / Heuristic Compliance:** Built using clean, statically linked code (Rust/C++) to minimize false-positive detections common with unpacked Python/PowerShell scripts.

### 7.3 System Compatibility
* **OS:** Windows 10, Windows 11 (build 19041+). Linux Proton / SteamDeck desktop environment supported via Wine CLI registration scripts.
* **Architecture:** x86_64, ARM64 (via emulation layer).

---

## 8. Detailed Edge Case Matrix & Error Recovery

| Scenario | Risk Level | Detection Mechanism | Recovery Protocol |
| :--- | :--- | :--- | :--- |
| **Game Executable Running** | High | Win32 API `EnumProcesses` checks for locks on target DLLs. | Display error toast: *"Cannot inject while game is running. Please close the game first."* |
| **Multiple Target DLLs Found** | Medium | Discovery engine finds DLLs in multiple subdirectories. | Prioritize deepest executable path or path matching active executable directory tree. |
| **No Internet Connection** | Low | Network API query times out after 1500ms. | Fall back immediately to offline PE metadata parsing or terminal manual prompt. |
| **Existing Goldberg Config Present** | Medium | Pre-existing `steam_settings` directory detected. | Backup existing settings into `steam_settings.bak_[timestamp]` before overwriting. |
| **Read-Only File Permissions** | Medium | File write test returns permission error. | Strip `READONLY` attribute flag using `SetFileAttributesW`. |
| **Non-Standard DLL Name** | Low | Game uses embedded or renamed Steam library (e.g., `steam_api64_orig.dll`). | Prompt user in terminal interface to manually select the target wrapper binary. |

---

## 9. UX & Command Line Interface (CLI) Specification

### 9.1 Context Menu Executable Flags
AutoGSE supports standard CLI commands for advanced users and scripts:

```bash
# Inject emulator automatically
autogse.exe inject --path "D:\Games\Elden Ring"

# Force explicit App ID override
autogse.exe inject --path "D:\Games\Elden Ring" --appid 1245620

# Revert folder to vanilla
autogse.exe revert --path "D:\Games\Elden Ring"

# Silent execution mode (no console popup unless error occurs)
autogse.exe inject --path "D:\Games\Elden Ring" --silent
```

### 9.2 OS Toast Notifications
AutoGSE triggers native Windows desktop notifications via WinRT Toast APIs:

* **Success Notification:**
  ```text
  [AutoGSE] Injection Complete
  Successfully linked achievements for Cyberpunk 2077 (AppID: 1091500).
  Ready for Achievement Watcher / Hydra Launcher.
  ```
* **Revert Notification:**
  ```text
  [AutoGSE] Rollback Complete
  Restored original steam_api64.dll and removed emulator configs.
  ```

---

## 10. Quality Assurance & Test Matrix

### 10.1 Tested Game Engine Formats
AutoGSE must be validated against a test suite of major game engine architectures prior to release:

```
[Test Matrix]
 ├── Unreal Engine 4 (e.g., Stray) -> Test binary in Engine/Binaries/Win64/
 ├── Unreal Engine 5 (e.g., Lords of the Fallen) -> Test binary in Hexworks/Binaries/Win64/
 ├── Unity x64 (e.g., Cult of the Lamb) -> Test binary in root folder
 ├── Unity x86 (Legacy) -> Test 32-bit emulator DLL swap
 ├── RE Engine (e.g., Resident Evil Village) -> Test inline DLL replacement
 └── Custom Launcher Hierarchy -> Test root launcher pointing to nested subfolder
```

---

## 11. Project Milestones & Development Roadmap

```
[Phase 1: Core Engine] --------> [Phase 2: Discovery & Scraper] ----> [Phase 3: Integration] -----> [Phase 4: Polish & Delivery]
- Rust/C++ CLI Framework         - PE Metadata Parser                 - GSE Tools Automation          - Windows Installer (InnoSetup)
- Win32 Context Menu Registry    - Steam Web API Client               - Achievement Watcher Hooks     - Desktop Toast Notifications
- File Manager & Mutex Backups   - Jaro-Winkler Fuzzy Engine          - Manifest State Machine         - Automated Test Suite Pass
```

---