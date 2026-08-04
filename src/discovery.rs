use std::collections::VecDeque;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::error::AutoGseError;
use crate::interaction::Interaction;

/// BFS depth cap beneath D_root (PRD §5.2.1).
const MAX_DEPTH: usize = 6;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

pub struct TargetResolution {
    pub tod: PathBuf,
    pub dll_path: PathBuf,
    #[allow(dead_code)] // recorded for future diagnostics/logging, not consumed yet
    pub depth: usize,
}

#[derive(Debug, Clone)]
struct DllMatch {
    path: PathBuf,
    depth: usize,
}

/// Path normalization (PRD §5.2.1): file -> D_root = Directory(P); directory
/// -> D_root = P. Exposed separately from `resolve_target` so callers can
/// take a lock on D_root *before* scanning — D_root is knowable immediately
/// from the raw CLI argument, unlike the TOD, which discovery itself
/// determines.
pub fn compute_d_root(path: &Path) -> Result<PathBuf, AutoGseError> {
    if path.is_file() {
        Ok(path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")))
    } else if path.is_dir() {
        Ok(path.to_path_buf())
    } else {
        Err(AutoGseError::TargetNotFound(path.to_path_buf()))
    }
}

/// Resolves a right-clicked file/folder to its Target Operating Directory and
/// the Steam API DLL within it (PRD §5.2). Replaces Phase 1's naive
/// `resolve_target_dir`/`find_target_dll` stand-ins with the real recursive
/// BFS discovery engine. `interaction` gates the non-standard-DLL-name
/// fallback prompt (Phase 7 §7.0): `None` behaves exactly like the old
/// `interactive: false` (never prompts, falls straight to `DllNotFound`).
pub fn resolve_target(path: &Path, interaction: Option<&dyn Interaction>) -> Result<TargetResolution, AutoGseError> {
    // Real bug found live: a real game (The Binding of Isaac: Rebirth) ships
    // its actual `isaac-ng.exe` and real `steam_api.dll` at the root, but
    // also bundles an unrelated helper tool (`tools\ModUploader\`) with its
    // own separate `steam_api.dll` deeper in the tree. `pick_best`'s pure
    // "deepest wins" rule (needed for the opposite, equally real UE4/5
    // pattern — decoy launcher at root, real DLL buried deep, no root DLL
    // at all) picked the bundled tool's copy instead — wrong, and not just
    // by heuristic: Windows' own DLL search order loads a DLL from the same
    // directory as the launched executable first, so when the caller points
    // at a *specific* .exe and a DLL sits right beside it, that is
    // unambiguously the one the running game will actually load, regardless
    // of what else exists deeper in the tree. Checked first, before depth
    // is even considered; falls through to the unchanged depth-based logic
    // below whenever `path` is a folder (no specific exe named) or no exact
    // match happens to sit beside the given file.
    let d_root = compute_d_root(path)?;

    let (exact, near) = scan(&d_root);

    if path.is_file() {
        if let Some(beside) = exact.iter().find(|m| m.path.parent() == Some(d_root.as_path())) {
            return Ok(to_resolution(beside, &d_root));
        }
    }

    if let Some(best) = pick_best(&exact) {
        return Ok(to_resolution(&best, &d_root));
    }

    if !near.is_empty() {
        if let Some(interaction) = interaction {
            if let Some(resolution) = prompt_non_standard_name(&near, &d_root, interaction) {
                return Ok(resolution);
            }
        }
    }

    Err(AutoGseError::DllNotFound(d_root))
}

/// One discoverable game target found under a fleet `--root` (Phase 6 §6.8).
pub struct FleetTarget {
    pub tod: PathBuf,
    #[allow(dead_code)] // recorded for future diagnostics/logging, not consumed yet
    pub dll_path: PathBuf,
}

/// Enumerates one target per immediate subdirectory of `root` — treats
/// `root` as a games-library folder (e.g. `SteamLibrary\steamapps\common\`)
/// whose *children* are individual game D_roots, each independently
/// BFS-scanned to the same `MAX_DEPTH` budget `resolve_target` uses for a
/// single game. This is deliberately **not** one BFS walk from `root`
/// itself: a deeply-nested game (e.g. depth 6 from its own folder) would
/// sit at depth 7+ from a shared library root and fall outside that budget,
/// silently vanishing from the scan. Subfolders with no discoverable DLL
/// are silently skipped — not every subfolder of a library root is
/// necessarily a Steam game. Never prompts (unlike `resolve_target`): a
/// non-standard DLL name in one of many targets shouldn't block scanning
/// the rest, so those are simply skipped rather than surfaced.
pub fn find_all_targets_under(root: &Path) -> Result<Vec<FleetTarget>, AutoGseError> {
    if !root.is_dir() {
        return Err(AutoGseError::TargetNotFound(root.to_path_buf()));
    }

    let mut targets = Vec::new();
    for entry in std::fs::read_dir(root)?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let (exact, _near) = scan(&path);
        if let Some(best) = pick_best(&exact) {
            let tod = best.path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.clone());
            targets.push(FleetTarget { tod, dll_path: best.path });
        }
    }
    targets.sort_by(|a, b| a.tod.cmp(&b.tod));
    Ok(targets)
}

fn to_resolution(m: &DllMatch, d_root: &Path) -> TargetResolution {
    TargetResolution {
        tod: m.path.parent().map(Path::to_path_buf).unwrap_or_else(|| d_root.to_path_buf()),
        dll_path: m.path.clone(),
        depth: m.depth,
    }
}

/// PRD §8's "non-standard/renamed DLL name" edge case: offers a manual pick
/// among near-miss `.dll` files (e.g. `steam_api64_orig.dll`), or a fully
/// manual path entry, via the caller-supplied `Interaction`. Returns `None`
/// on cancel/EOF so the caller falls back to the ordinary `DllNotFound`
/// error.
fn prompt_non_standard_name(near: &[DllMatch], d_root: &Path, interaction: &dyn Interaction) -> Option<TargetResolution> {
    let paths: Vec<PathBuf> = near.iter().map(|m| m.path.clone()).collect();
    let picked = interaction.pick_dll(&paths, d_root)?;
    // Preserve the real depth when the picked path was one of the scanned
    // near-matches (list selection); a manually-typed path has no scanned
    // depth to recover, so it defaults to 0.
    let depth = near.iter().find(|m| m.path == picked).map(|m| m.depth).unwrap_or(0);
    Some(to_resolution(&DllMatch { path: picked, depth }, d_root))
}

/// BFS beneath `d_root` to `MAX_DEPTH`, collecting exact `steam_api(64).dll`
/// matches and "near" matches (any `.dll` containing `steam_api`) for the
/// non-standard-name fallback, in a single walk.
fn scan(d_root: &Path) -> (Vec<DllMatch>, Vec<DllMatch>) {
    let mut exact = Vec::new();
    let mut near = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((d_root.to_path_buf(), 0));

    while let Some((dir, depth)) = queue.pop_front() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            if metadata.is_dir() {
                let is_reparse_point = metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
                if depth < MAX_DEPTH && !is_reparse_point {
                    queue.push_back((entry.path(), depth + 1));
                }
            } else if metadata.is_file() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name == "steam_api.dll" || name == "steam_api64.dll" {
                    exact.push(DllMatch { path: entry.path(), depth });
                } else if name.ends_with(".dll") && name.contains("steam_api") {
                    near.push(DllMatch { path: entry.path(), depth });
                }
            }
        }
    }

    (exact, near)
}

/// PRD §8: multiple matches prioritize the deepest path (matches the UE4/5
/// examples where the real DLL sits several levels below a decoy launcher);
/// ties at the same depth are broken lexicographically for determinism,
/// since the PRD specifies no further tiebreaker.
fn pick_best(matches: &[DllMatch]) -> Option<DllMatch> {
    let mut sorted: Vec<&DllMatch> = matches.iter().collect();
    sorted.sort_by(|a, b| {
        b.depth
            .cmp(&a.depth)
            .then_with(|| a.path.to_string_lossy().to_lowercase().cmp(&b.path.to_string_lossy().to_lowercase()))
    });
    sorted.into_iter().next().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"placeholder").unwrap();
    }

    #[test]
    fn ue4_ue5_nested_binary_is_found() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("Cyberpunk.exe"));
        touch(&dir.path().join("Engine/Binaries/Win64/steam_api64.dll"));

        let resolution = resolve_target(&dir.path().join("Cyberpunk.exe"), None).unwrap();

        assert_eq!(resolution.tod, dir.path().join("Engine/Binaries/Win64"));
        assert_eq!(resolution.depth, 3);
    }

    #[test]
    fn unity_flat_root_is_found() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("HollowKnight.exe"));
        touch(&dir.path().join("steam_api.dll"));

        let resolution = resolve_target(&dir.path().join("HollowKnight.exe"), None).unwrap();

        assert_eq!(resolution.tod, dir.path().to_path_buf());
        assert_eq!(resolution.depth, 0);
    }

    #[test]
    fn re_engine_flat_root_is_found() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("RE2.exe"));
        touch(&dir.path().join("steam_api64.dll"));

        let resolution = resolve_target(&dir.path().join("RE2.exe"), None).unwrap();

        assert_eq!(resolution.tod, dir.path().to_path_buf());
    }

    #[test]
    fn custom_nested_launcher_is_found() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("Launcher.exe"));
        touch(&dir.path().join("bin/x64/steam_api64.dll"));

        let resolution = resolve_target(&dir.path().join("Launcher.exe"), None).unwrap();

        assert_eq!(resolution.tod, dir.path().join("bin/x64"));
    }

    #[test]
    fn dll_beside_a_specifically_given_exe_wins_over_a_deeper_bundled_tool_copy() {
        // Real bug, found live against a real game (The Binding of Isaac:
        // Rebirth): a bundled helper tool (`tools/ModUploader/`) ships its
        // own separate steam_api.dll deeper than the real one at the root,
        // beside the actual game exe. The opposite depth relationship from
        // `deepest_match_wins_over_decoy` below — same shape of conflict,
        // opposite correct answer, which is exactly why this needs a
        // real specific-exe signal rather than depth alone.
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("isaac-ng.exe"));
        touch(&dir.path().join("steam_api.dll")); // real, beside the given exe, depth 0
        touch(&dir.path().join("tools/ModUploader/steam_api.dll")); // decoy bundled tool, depth 2

        let resolution = resolve_target(&dir.path().join("isaac-ng.exe"), None).unwrap();

        assert_eq!(resolution.tod, dir.path());
        assert_eq!(resolution.depth, 0);
    }

    #[test]
    fn folder_only_input_still_prefers_the_deepest_match_even_with_a_root_dll() {
        // The fix above only applies when the caller names a specific exe.
        // Given just the folder (no specific exe known), the original
        // deepest-wins behavior must still stand — this is the exact
        // scenario `deepest_match_wins_over_decoy` below already covers,
        // duplicated here under a name that makes the folder-vs-file
        // distinction explicit, so a future change to one doesn't
        // accidentally regress the other without a renamed test failing too.
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("steam_api64.dll"));
        touch(&dir.path().join("Engine/Binaries/Win64/steam_api64.dll"));

        let resolution = resolve_target(dir.path(), None).unwrap();

        assert_eq!(resolution.tod, dir.path().join("Engine/Binaries/Win64"));
    }

    #[test]
    fn deepest_match_wins_over_decoy() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("steam_api64.dll")); // decoy at depth 0
        touch(&dir.path().join("Engine/Binaries/Win64/steam_api64.dll")); // real, depth 3

        let resolution = resolve_target(dir.path(), None).unwrap();

        assert_eq!(resolution.depth, 3);
        assert_eq!(resolution.tod, dir.path().join("Engine/Binaries/Win64"));
    }

    #[test]
    fn same_depth_tie_breaks_lexicographically() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("A/steam_api64.dll"));
        touch(&dir.path().join("B/steam_api64.dll"));

        let resolution = resolve_target(dir.path(), None).unwrap();

        assert_eq!(resolution.tod, dir.path().join("A"));
    }

    #[test]
    fn depth_boundary_is_inclusive_at_six_exclusive_at_seven() {
        let dir = TempDir::new().unwrap();
        let depth6 = dir.path().join("L1/L2/L3/L4/L5/L6");
        touch(&depth6.join("steam_api64.dll"));
        touch(&depth6.join("L7/steam_api64.dll")); // depth 7, must be unreachable

        let resolution = resolve_target(dir.path(), None).unwrap();

        assert_eq!(resolution.depth, 6);
        assert_eq!(resolution.tod, depth6);
    }

    #[test]
    fn non_standard_dll_name_yields_near_match_not_exact() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("steam_api64_orig.dll"));

        let (exact, near) = scan(dir.path());

        assert!(exact.is_empty());
        assert_eq!(near.len(), 1);
    }

    #[test]
    fn non_standard_dll_name_is_dll_not_found_when_non_interactive() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("steam_api64_orig.dll"));

        let result = resolve_target(dir.path(), None);

        assert!(matches!(result, Err(AutoGseError::DllNotFound(_))));
    }

    #[test]
    fn no_dll_anywhere_is_dll_not_found() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("SomeGame.exe"));

        let result = resolve_target(dir.path(), None);

        assert!(matches!(result, Err(AutoGseError::DllNotFound(_))));
    }

    #[test]
    fn nonexistent_path_is_target_not_found() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does_not_exist");

        let result = resolve_target(&missing, None);

        assert!(matches!(result, Err(AutoGseError::TargetNotFound(_))));
    }

    #[test]
    fn find_all_targets_under_discovers_one_target_per_subfolder() {
        let root = TempDir::new().unwrap();
        touch(&root.path().join("GameA/steam_api64.dll"));
        touch(&root.path().join("GameB/Engine/Binaries/Win64/steam_api.dll"));
        touch(&root.path().join("NotAGame/readme.txt"));

        let mut targets = find_all_targets_under(root.path()).unwrap();
        targets.sort_by(|a, b| a.tod.cmp(&b.tod));

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].tod, root.path().join("GameA"));
        assert_eq!(targets[1].tod, root.path().join("GameB/Engine/Binaries/Win64"));
    }

    #[test]
    fn find_all_targets_under_errors_on_nonexistent_root() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does_not_exist");
        assert!(matches!(find_all_targets_under(&missing), Err(AutoGseError::TargetNotFound(_))));
    }

    #[test]
    fn cyclic_junction_does_not_hang_or_get_traversed() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("real/decoy.txt"));
        let junction = dir.path().join("junction_back_to_root");

        // Directory junctions don't require elevation/Developer Mode (unlike
        // symlinks), so this exercises the real Win32 reparse-point check.
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J should succeed without elevation");

        let start = std::time::Instant::now();
        let result = resolve_target(dir.path(), None);
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "BFS must terminate promptly despite a cyclic junction");
        assert!(matches!(result, Err(AutoGseError::DllNotFound(_))));
    }
}
