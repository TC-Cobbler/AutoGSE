use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::AutoGseError;
use crate::pe::Arch;

const GENERATE_EMU_CONFIG_TIMEOUT_ANON: Duration = Duration::from_secs(60);

/// `generate_interfaces(64).exe` is a small, local, non-networked binary
/// analysis tool — confirmed live to finish in well under a second against
/// a real DLL, so this is generous headroom, not a real budget.
const GENERATE_INTERFACES_TIMEOUT: Duration = Duration::from_secs(20);

/// Longer than the anonymous budget: a real login plus a possible
/// interactive Steam Guard code entry (relayed through the inherited
/// console, see `login_prompt.rs`) can legitimately take a couple of
/// minutes of human time, not just network latency.
const GENERATE_EMU_CONFIG_TIMEOUT_AUTH: Duration = Duration::from_secs(180);

/// Resolved Steam access mode for one `generate_emu_config.exe` invocation.
/// `Anonymous` is Phase 3's original, unchanged path (`-anon -skip_ach`).
/// `Authenticated` is Phase 5: real credentials via env vars (never a
/// plaintext `my_login.txt` on disk), simply omitting `-anon` (not `-tok` —
/// see `run_generate_emu_config`'s own comment for why that flag is
/// deliberately never passed), and `-acw` re-enabled since achievement data
/// — confirmed live to hang under anonymous login regardless of `-acw`
/// specifically — needs a real account.
pub enum AuthMode {
    Anonymous,
    Authenticated { username: String, password: String },
}

/// Resolves the vendored `alex47exe/gse_fork` tools directory (`GEC_ROOT` —
/// the folder containing `generate_emu_config.exe`, `_DEFAULT/`, etc).
///
/// Debug builds resolve it at compile time relative to this repo, so
/// `cargo run`/`cargo test` work regardless of invocation CWD. Release
/// builds expect it vendored beside the shipped `autogse.exe` as
/// `gen_emu_cfg\` — **this is the seam Phase 4's installer must fill**; Phase
/// 3 doesn't build that installer, it just documents the contract.
#[cfg(debug_assertions)]
pub fn tools_root() -> Result<PathBuf, AutoGseError> {
    let dev_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("alex47exe-gse_fork/gen_emu_cfg-Windows-Release/generate_emu_config");
    if dev_path.is_dir() {
        Ok(dev_path)
    } else {
        Err(AutoGseError::VendoredToolsNotFound(dev_path))
    }
}

#[cfg(not(debug_assertions))]
pub fn tools_root() -> Result<PathBuf, AutoGseError> {
    let exe_dir = std::env::current_exe()?.parent().map(Path::to_path_buf).unwrap_or_default();
    let release_path = exe_dir.join("gen_emu_cfg");
    if release_path.is_dir() {
        Ok(release_path)
    } else {
        Err(AutoGseError::VendoredToolsNotFound(release_path))
    }
}

/// Resolves the real Goldberg emulator DLL matching `arch`, from the
/// vendored `_DEFAULT/0/` base payload (confirmed by direct inspection to be
/// the actual emulator binaries, not a placeholder).
pub fn dll_source_path(arch: Arch) -> Result<PathBuf, AutoGseError> {
    let filename = match arch {
        Arch::X86 => "steam_api.dll",
        Arch::X64 => "steam_api64.dll",
    };
    let path = tools_root()?.join("_DEFAULT").join("0").join(filename);
    if path.is_file() {
        Ok(path)
    } else {
        Err(AutoGseError::VendoredToolsNotFound(path))
    }
}

/// Spawns `cmd`, polls non-blockingly until it exits or `timeout` elapses
/// (killing it on timeout). Mandatory for every external-tool invocation in
/// this module: `generate_emu_config.exe`'s Steam login and `au3.exe`'s
/// interpreter are both opaque, network-and/or-3rd-party-dependent
/// processes whose failure modes can be an indefinite hang, not just a
/// clean error exit (confirmed empirically for achievement-data fetching).
///
/// Both stdout and stderr are piped, each through a byte-level tee: relayed
/// immediately to AutoGSE's own inherited stdout/stderr (so live progress
/// and, critically, a no-trailing-newline interactive prompt like Steam
/// Guard's code entry still appear in real time — a line-buffered relay
/// would silently swallow such a prompt until the *next* newline, which
/// never comes until the human already responded) while simultaneously
/// captured for the error message on a nonzero exit. stdin stays inherited
/// throughout, untouched, so typed responses still reach the child exactly
/// as before this existed.
///
/// Confirmed live that `generate_emu_config.exe` writes its normal progress
/// to stdout; on at least one real failure (exit code 1, cause still
/// unknown) stderr was empty, so stdout must be captured too, not just
/// stderr — a bare exit code with nothing else was reported until this.
///
/// Reading the captured output is bounded by `DRAIN_TIMEOUT`, never an
/// unconditional join: `child.kill()`/a clean exit only guarantees the
/// *direct* child is gone, not any grandchild process it spawned that
/// inherited the same pipe handles (confirmed empirically — a killed `cmd
/// /C` wrapping a still-running `ping` left its pipes' write ends open for
/// the ping's full remaining runtime, since Windows doesn't kill process
/// trees by default). An unconditionally-joined reader thread would've
/// reintroduced exactly the indefinite-hang failure mode this whole
/// function exists to prevent — capturing output must never risk that.
pub(crate) fn run_with_timeout(mut cmd: Command, timeout: Duration, tool_name: &str) -> Result<(), AutoGseError> {
    const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| AutoGseError::ExternalToolFailed { tool: tool_name.to_string(), message: format!("failed to spawn: {e}") })?;

    let stdout_pipe = child.stdout.take().expect("stdout was piped above");
    let stderr_pipe = child.stderr.take().expect("stderr was piped above");
    let stdout_rx = spawn_tee(stdout_pipe, std::io::stdout());
    let stderr_rx = spawn_tee(stderr_pipe, std::io::stderr());

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let _ = stdout_rx.recv_timeout(DRAIN_TIMEOUT);
                let _ = stderr_rx.recv_timeout(DRAIN_TIMEOUT);
                return Ok(());
            }
            Ok(Some(status)) => {
                let out = stdout_rx.recv_timeout(DRAIN_TIMEOUT).unwrap_or_default();
                let err = stderr_rx.recv_timeout(DRAIN_TIMEOUT).unwrap_or_default();
                let mut detail = tail_lines(&out, 15);
                let err_tail = tail_lines(&err, 10);
                if !err_tail.is_empty() {
                    if !detail.is_empty() {
                        detail.push('\n');
                    }
                    detail.push_str(&err_tail);
                }
                let message =
                    if detail.is_empty() { format!("exited with {status}") } else { format!("exited with {status}\n{detail}") };
                return Err(AutoGseError::ExternalToolFailed { tool: tool_name.to_string(), message });
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Deliberately not draining output here (see doc
                    // comment above) — a timed-out process is exactly the
                    // case most likely to have orphaned a pipe-holding
                    // grandchild behind.
                    return Err(AutoGseError::ExternalToolTimeout(tool_name.to_string()));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(AutoGseError::ExternalToolFailed { tool: tool_name.to_string(), message: format!("wait failed: {e}") });
            }
        }
    }
}

/// Reads `pipe` in raw byte chunks (not line-buffered — see the doc comment
/// on `run_with_timeout` for why), writing each chunk immediately to
/// `relay_to` and also accumulating it, sending the full accumulated bytes
/// once `pipe` hits EOF.
fn spawn_tee<R, W>(mut pipe: R, mut relay_to: W) -> std::sync::mpsc::Receiver<String>
where
    R: std::io::Read + Send + 'static,
    W: std::io::Write + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = relay_to.write_all(&buf[..n]);
                    let _ = relay_to.flush();
                    captured.extend_from_slice(&buf[..n]);
                }
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&captured).into_owned());
    });
    rx
}

/// Last `max_lines` non-empty lines of `s`, trimmed — keeps a failing
/// child's captured output from ballooning an error message/toast with a
/// full traceback when just the tail usually says enough.
fn tail_lines(s: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

/// Runs the real `generate_emu_config.exe` for `app_id`, writing its output
/// directly into `out_dir` (`-rel_raw`, confirmed empirically to honor
/// `Command::current_dir`).
///
/// Deliberately **omits `-acw`** (Achievement Watcher schema generation)
/// **and passes `-skip_ach`**: both were found, via direct empirical testing
/// (multiple runs, both Git Bash and native PowerShell process invocation,
/// no orphaned process left behind), to hang indefinitely under anonymous
/// login for achievement-data fetching — a confirmed tool/environment
/// limitation, not a bug in this wrapper. Achievement Watcher integration
/// (PRD §6/roadmap 3.4) is descoped from Phase 3 pending investigation.
///
/// `gse_generate_interfaces` (PRD §5.4.1 step 2, producing `steam_interfaces.txt`)
/// is likewise **not wired up**: it fails silently (exit 1, no output) even
/// when staged and invoked exactly as the reference `.bat` workflow does,
/// for a root cause not yet identified (suspected `@ScriptDir`/`@ScriptName`
/// resolution difference between a compiled standalone `.a3x` and one run
/// via `au3.exe /AutoIt3ExecuteScript`). Goldberg's emulator works with
/// default interface versions without this file — it's a compatibility
/// optimization, not a hard requirement — so this is deferred rather than
/// blocking the core inject/revert pipeline. `-skip_con`/`-skip_inv` skip
/// controller/inventory data as unnecessary for achievement injection.
///
/// `auth` selects between Phase 3's unchanged anonymous path and Phase 5's
/// authenticated one (just omitting `-anon`, plus `-acw`; credentials
/// passed as env vars on this child process only — confirmed via the
/// vendored tool's own README that `GSE_CFG_USERNAME`/`GSE_CFG_PASSWORD`
/// override a `my_login.txt` file, so AutoGSE uses only the env-var
/// mechanism and never writes that file to disk).
pub fn run_generate_emu_config(app_id: u64, out_dir: &Path, auth: &AuthMode) -> Result<(), AutoGseError> {
    let exe = tools_root()?.join("generate_emu_config.exe");
    let mut cmd = Command::new(&exe);
    cmd.args(["-rel_raw", "-clr", "-skip_con", "-skip_inv"]);
    // `run_with_timeout` now pipes this process's stdout/stderr (to relay
    // *and* capture it) instead of leaving them inherited; a piped (non-tty)
    // stream makes CPython default to block-buffered output, which would
    // otherwise stall a no-trailing-newline prompt like Steam Guard's code
    // entry until an unrelated later flush. This forces unbuffered I/O so
    // the relay stays real-time regardless.
    cmd.env("PYTHONUNBUFFERED", "1");

    let timeout = match auth {
        AuthMode::Anonymous => {
            cmd.args(["-anon", "-skip_ach"]);
            GENERATE_EMU_CONFIG_TIMEOUT_ANON
        }
        AuthMode::Authenticated { username, password } => {
            // Deliberately no `-tok`: confirmed live (this phase) that it
            // makes the tool try to write `refresh_tokens.json` beside its
            // own exe — `tools_root()`, which in a real install is
            // `C:\Program Files\AutoGSE\gen_emu_cfg\`, not writable by a
            // normal (non-elevated) process, causing a guaranteed
            // `PermissionError` on every authenticated run from a real
            // install. `-tok` only caches the login session for reuse
            // (confirmed from the tool's own `--help`: real-vs-anonymous
            // login is controlled purely by `-anon`'s absence); AutoGSE
            // always supplies fresh credentials via env vars on every
            // invocation anyway, so that cache is never relied upon.
            cmd.args(["-acw"]).env("GSE_CFG_USERNAME", username).env("GSE_CFG_PASSWORD", password);
            GENERATE_EMU_CONFIG_TIMEOUT_AUTH
        }
    };

    cmd.arg(app_id.to_string()).current_dir(out_dir);
    run_with_timeout(cmd, timeout, "generate_emu_config.exe")
}

/// Bypasses the vendored `gse_generate_interfaces` AutoIt-orchestrated
/// wrapper entirely. Confirmed live (this phase, via direct testing against
/// a real, legitimately-owned game's DLL) that the AutoIt chain — both
/// `au3.exe /AutoIt3ExecuteScript` and the renamed-interpreter
/// same-basename-companion convention — silently produces nothing even with
/// a real original DLL and a working `generate_interfaces(64).exe` both
/// present. The underlying `generate_interfaces(64).exe` binary itself,
/// however, works correctly when invoked directly — this reimplements the
/// small amount of post-processing `generate_interfaces.au3` did around it
/// (move `steam_interfaces.txt` into `steam_settings/`, write a
/// `[steam_interfaces]`-headed `.ini` copy) natively, instead of depending
/// on the broken AutoIt layer. The CODEX/RUNE-specific interface remapping
/// further down that same script is deliberately not ported — that's for
/// other cracking tools, irrelevant to Goldberg.
///
/// `original_dll_path` must be the real, pre-swap game DLL (AutoGSE's own
/// `.org` backup, via `backup::ensure_backed_up`) — this tool extracts
/// interface version strings out of *that*, not out of anything
/// `generate_emu_config.exe` downloads.
///
/// Returns `Ok(false)` (not an error) if the tool binary is missing —
/// observed intermittently during development, likely antivirus
/// quarantining a freshly-copied `generate_interfaces(64).exe` (it's a
/// DLL-interface-extraction tool, a common heuristic false-positive/PUA
/// pattern) — or if it ran but produced nothing. This step is a
/// compatibility optimization, not a hard requirement, matching
/// `run_generate_emu_config`'s own established philosophy for `-acw`.
pub fn generate_interfaces(out_dir: &Path, arch: Arch, original_dll_path: &Path) -> Result<bool, AutoGseError> {
    let exe_name = match arch {
        Arch::X86 => "generate_interfaces.exe",
        Arch::X64 => "generate_interfaces64.exe",
    };
    let tools_dir = out_dir.join("steam_misc").join("tools").join("generate_interfaces");
    let exe = tools_dir.join(exe_name);

    // `generate_emu_config.exe`'s own preset copy only places the archive
    // (`generate_interfaces.7z`), not the extracted binaries — confirmed by
    // direct inspection of `_DEFAULT/0/steam_misc/tools/generate_interfaces/`.
    // The vendored AutoIt script normally extracts it on demand via `7za.exe`;
    // do the same here since we're bypassing that script entirely.
    if !exe.is_file() {
        let archive = tools_dir.join("generate_interfaces.7z");
        let sevenzip = out_dir.join("steam_misc").join("tools").join("7za").join("7za.exe");
        if !archive.is_file() || !sevenzip.is_file() {
            return Ok(false);
        }
        let mut extract_cmd = Command::new(&sevenzip);
        extract_cmd.arg("x").arg(&archive).arg(format!("-o{}", tools_dir.display())).arg("-aoa");
        if run_with_timeout(extract_cmd, GENERATE_INTERFACES_TIMEOUT, "7za.exe").is_err() || !exe.is_file() {
            return Ok(false);
        }
    }

    let mut cmd = Command::new(&exe);
    cmd.arg(original_dll_path).current_dir(out_dir);
    if run_with_timeout(cmd, GENERATE_INTERFACES_TIMEOUT, exe_name).is_err() {
        return Ok(false);
    }

    let produced = out_dir.join("steam_interfaces.txt");
    if !produced.is_file() {
        return Ok(false);
    }

    let dest_dir = out_dir.join("steam_settings");
    std::fs::create_dir_all(&dest_dir)?;
    let dest_txt = dest_dir.join("steam_interfaces.txt");
    std::fs::rename(&produced, &dest_txt)?;

    let content = std::fs::read_to_string(&dest_txt)?;
    std::fs::write(dest_dir.join("steam_interfaces.ini"), format!("[steam_interfaces]\r\n{content}"))?;

    Ok(true)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), AutoGseError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn list_files_relative(root: &Path) -> Result<Vec<String>, AutoGseError> {
    let mut out = Vec::new();
    list_files_relative_into(root, Path::new(""), &mut out)?;
    Ok(out)
}

fn list_files_relative_into(base: &Path, rel: &Path, out: &mut Vec<String>) -> Result<(), AutoGseError> {
    for entry in std::fs::read_dir(base.join(rel))? {
        let entry = entry?;
        let child_rel = rel.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            list_files_relative_into(base, &child_rel, out)?;
        } else {
            out.push(child_rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// Merges `gec_out/steam_settings/*` into `tod/steam_settings/*`, returning
/// every copied file's TOD-relative path (e.g. `steam_settings/configs.main.ini`)
/// for the manifest's `injected_files[]` — an accumulated, not hardcoded,
/// list, since the exact file set varies per game/tool-output shape.
pub fn merge_steam_settings(gec_out: &Path, tod: &Path) -> Result<Vec<String>, AutoGseError> {
    let src = gec_out.join("steam_settings");
    let dst = tod.join("steam_settings");
    copy_dir_recursive(&src, &dst)?;
    let files = list_files_relative(&dst)?;
    Ok(files.into_iter().map(|f| format!("steam_settings/{f}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn tools_root_resolves_to_real_vendored_tree() {
        let root = tools_root().unwrap();
        assert!(root.join("generate_emu_config.exe").is_file());
        assert!(root.join("_DEFAULT").join("0").is_dir());
    }

    #[test]
    fn dll_source_path_resolves_both_arches() {
        assert!(dll_source_path(Arch::X64).unwrap().is_file());
        assert!(dll_source_path(Arch::X86).unwrap().is_file());
    }


    #[test]
    fn generate_interfaces_returns_false_when_tool_and_archive_both_missing() {
        let dir = TempDir::new().unwrap();
        let dll = dir.path().join("fake.dll");
        std::fs::write(&dll, b"not a real dll").unwrap();
        assert_eq!(generate_interfaces(dir.path(), Arch::X64, &dll).unwrap(), false);
    }

    /// Manual QA only (live, requires a real Steam game's original DLL, set
    /// via `AUTOGSE_TEST_DLL_PATH`, e.g. a `steam_api64.dll` copied out of an
    /// installed game before AutoGSE ever touches it):
    /// `cargo test goldberg::tests::live_generate_interfaces -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_generate_interfaces() {
        let dll_path = std::env::var("AUTOGSE_TEST_DLL_PATH").expect("set AUTOGSE_TEST_DLL_PATH to a real steam_api64.dll");
        let out_dir = TempDir::new().unwrap();
        // Mirrors exactly what `generate_emu_config.exe`'s own preset-0 copy
        // places in a real `out_dir`: the `.7z` archive, not pre-extracted.
        let tools_src = tools_root().unwrap().join("_DEFAULT").join("0").join("steam_misc").join("tools");
        let tools_dst = out_dir.path().join("steam_misc").join("tools");
        copy_dir_recursive(&tools_src.join("generate_interfaces"), &tools_dst.join("generate_interfaces")).unwrap();
        copy_dir_recursive(&tools_src.join("7za"), &tools_dst.join("7za")).unwrap();

        let produced = generate_interfaces(out_dir.path(), Arch::X64, Path::new(&dll_path)).unwrap();
        assert!(produced);
        assert!(out_dir.path().join("steam_settings/steam_interfaces.txt").is_file());
        assert!(out_dir.path().join("steam_settings/steam_interfaces.ini").is_file());
    }

    #[test]
    fn run_with_timeout_succeeds_fast() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "exit", "0"]);
        run_with_timeout(cmd, Duration::from_secs(10), "cmd").unwrap();
    }

    #[test]
    fn run_with_timeout_reports_nonzero_exit() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "exit", "1"]);
        let result = run_with_timeout(cmd, Duration::from_secs(10), "cmd");
        assert!(matches!(result, Err(AutoGseError::ExternalToolFailed { .. })));
    }

    #[test]
    fn run_with_timeout_kills_and_errors_on_deadline() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "ping", "-n", "30", "127.0.0.1", ">", "NUL"]);
        let start = Instant::now();
        let result = run_with_timeout(cmd, Duration::from_millis(500), "cmd");
        assert!(matches!(result, Err(AutoGseError::ExternalToolTimeout(_))));
        assert!(start.elapsed() < Duration::from_secs(5), "must not wait anywhere near the full hung-process duration");
    }

    fn touch(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }


    #[test]
    fn merge_steam_settings_copies_and_tracks_files() {
        let gec_out = TempDir::new().unwrap();
        let tod = TempDir::new().unwrap();
        touch(&gec_out.path().join("steam_settings/configs.main.ini"), b"a");
        touch(&gec_out.path().join("steam_settings/controller/glyphs/button_a.png"), b"b");

        let mut tracked = merge_steam_settings(gec_out.path(), tod.path()).unwrap();
        tracked.sort();

        assert_eq!(
            tracked,
            vec!["steam_settings/configs.main.ini".to_string(), "steam_settings/controller/glyphs/button_a.png".to_string()]
        );
        assert!(tod.path().join("steam_settings/configs.main.ini").is_file());
        assert!(tod.path().join("steam_settings/controller/glyphs/button_a.png").is_file());
    }

    /// Manual QA only (live, ~5-60s, real network + real external process):
    /// `cargo test goldberg::tests::live_run_generate_emu_config -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_run_generate_emu_config() {
        let out_dir = TempDir::new().unwrap();
        run_generate_emu_config(480, out_dir.path(), &AuthMode::Anonymous).unwrap(); // 480 = Spacewar, Valve's public test app
        assert!(out_dir.path().join("steam_settings/configs.main.ini").is_file());
        assert!(out_dir.path().join("steam_settings/steam_appid.txt").is_file());
        assert!(out_dir.path().join("steam_misc/tools/au3/au3.exe").is_file());
    }

    /// Manual QA only, requires real Steam credentials via
    /// `AUTOGSE_TEST_STEAM_USERNAME`/`AUTOGSE_TEST_STEAM_PASSWORD` env vars:
    /// `cargo test goldberg::tests::live_run_generate_emu_config_authenticated -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_run_generate_emu_config_authenticated() {
        let username = std::env::var("AUTOGSE_TEST_STEAM_USERNAME").expect("set AUTOGSE_TEST_STEAM_USERNAME");
        let password = std::env::var("AUTOGSE_TEST_STEAM_PASSWORD").expect("set AUTOGSE_TEST_STEAM_PASSWORD");
        let out_dir = TempDir::new().unwrap();
        run_generate_emu_config(105600, out_dir.path(), &AuthMode::Authenticated { username, password }).unwrap(); // 105600 = Terraria
        assert!(out_dir.path().join("steam_settings/achievements.json").is_file());
        assert!(out_dir.path().join("steam_settings/img").is_dir());
    }
}
