using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.Win32;
using Playnite.SDK;
using Playnite.SDK.Models;
using Playnite.SDK.Plugins;

namespace AutoGSEPlaynitePlugin
{
    /// <summary>
    /// Mirrors <c>engine::JsonTarget</c> in the Rust CLI (src/engine.rs) —
    /// field names here are the same stable, snake_case contract `autogse
    /// list --json`/`autogse scan --json` emit. Keep in sync with that
    /// struct; do not rename fields independently on either side.
    /// </summary>
    public class AutoGseJsonTarget
    {
        [JsonPropertyName("path")]
        public string Path { get; set; }

        [JsonPropertyName("status")]
        public string Status { get; set; }

        [JsonPropertyName("mode")]
        public string Mode { get; set; }

        [JsonPropertyName("app_id")]
        public ulong? AppId { get; set; }

        [JsonPropertyName("game_title")]
        public string GameTitle { get; set; }
    }

    /// <summary>
    /// Phase 11 §11.4 skeleton: shells out to the real <c>autogse.exe</c>
    /// rather than reimplementing any injection logic in C#. Deliberately
    /// thin — this is a scaffold proving the integration seam (the CLI's
    /// <c>--json</c> output, added alongside this file) works end to end,
    /// not a polished, store-submitted extension. Structurally complete but
    /// only live-verified inside a real running Playnite install with a real
    /// game library, which this development environment may or may not have
    /// set up for building/deploying .NET extensions — see this folder's
    /// README for the manual build/deploy steps.
    /// </summary>
    public class AutoGsePlugin : GenericPlugin
    {
        private static readonly ILogger Logger = LogManager.GetLogger();

        public override Guid Id { get; } = Guid.Parse("8f2f0a0a-2e6a-4a9a-9a0b-1c9c3b2f7a11");

        public AutoGsePlugin(IPlayniteAPI api) : base(api)
        {
            Properties = new GenericPluginProperties { HasSettings = false };
        }

        /// <summary>
        /// Inno Setup's own auto-generated uninstall registry key, keyed on
        /// installer/autogse.iss's real <c>AppId</c> GUID
        /// (<c>{BDD72098-6E2A-48C7-9539-8DEC14FC937F}</c>) — every Inno
        /// Setup installer writes an <c>InstallLocation</c> value here
        /// unprompted, so this is a real, reliable way to find a genuine
        /// installed copy without needing AUTOGSE_EXE set at all. Confirmed
        /// live against a real install on this machine (`reg query`), not
        /// assumed from Inno Setup's docs alone.
        /// </summary>
        private const string InnoSetupUninstallKey =
            @"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{BDD72098-6E2A-48C7-9539-8DEC14FC937F}_is1";

        /// <summary>
        /// Resolution order: (1) the AUTOGSE_EXE environment variable, an
        /// explicit override; (2) the real installed location, read from
        /// Inno Setup's own uninstall registry key — covers the common case
        /// (a real AutoGSE install) with zero configuration; (3) bare
        /// "autogse.exe", relying on PATH — AutoGSE's own installer does not
        /// add itself to PATH, so this last resort will usually fail, but is
        /// kept as a no-worse-than-before fallback for anyone who put it on
        /// PATH manually.
        /// </summary>
        private static string AutoGseExePath()
        {
            var envOverride = Environment.GetEnvironmentVariable("AUTOGSE_EXE");
            if (!string.IsNullOrEmpty(envOverride))
            {
                return envOverride;
            }

            // Real bug found live and confirmed fixed: the ambient
            // `Registry.LocalMachine` (process-native view) came up empty
            // inside Playnite's process even though a standalone net462
            // console app, same user, same key, found it immediately — a
            // real, reproducible discrepancy, root cause not fully pinned
            // down (process-bitness inference said Playnite is 64-bit via
            // two independent checks — tasklist's "*32" marker and the
            // loaded ntdll.dll path — yet the native-view lookup still came
            // up empty). Rather than rely on that inference, this tries
            // both registry views explicitly and uses whichever finds it —
            // confirmed live this resolves it regardless of the underlying
            // cause.
            foreach (var view in new[] { RegistryView.Registry64, RegistryView.Registry32 })
            {
                try
                {
                    using (var hive = RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, view))
                    using (var key = hive.OpenSubKey(InnoSetupUninstallKey))
                    {
                        var installLocation = key?.GetValue("InstallLocation") as string;
                        if (!string.IsNullOrEmpty(installLocation))
                        {
                            var candidate = System.IO.Path.Combine(installLocation, "autogse.exe");
                            if (File.Exists(candidate))
                            {
                                return candidate;
                            }
                        }
                    }
                }
                catch (Exception ex)
                {
                    // Registry access can fail in ways specific to the host
                    // environment (permissions, corrupted key, etc.) — none
                    // of them should crash a status check; fall through to
                    // the PATH-based last resort below instead.
                    Logger.Warn(ex, $"AutoGSE: [{view}] could not read the install-location registry key.");
                }
            }

            return "autogse.exe";
        }

        /// <summary>
        /// Wraps the real <see cref="Win32Exception"/> Process.Start throws
        /// when the resolved exe genuinely doesn't exist (e.g. "The system
        /// cannot find the file specified") with a message that actually
        /// says what to do about it — Playnite's own generic "failed to
        /// execute menu action" wrapper around the raw Win32Exception gives
        /// no actionable hint otherwise, confirmed by hitting this exact
        /// failure live before AutoGseExePath() gained the registry lookup
        /// above.
        /// </summary>
        private static (int ExitCode, string Stdout, string Stderr) RunAutoGse(string arguments)
        {
            string exePath = AutoGseExePath();
            var psi = new ProcessStartInfo(exePath, arguments)
            {
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
            };

            try
            {
                using (var proc = Process.Start(psi))
                {
                    string stdout = proc.StandardOutput.ReadToEnd();
                    string stderr = proc.StandardError.ReadToEnd();
                    proc.WaitForExit();
                    return (proc.ExitCode, stdout, stderr);
                }
            }
            catch (Win32Exception ex)
            {
                throw new InvalidOperationException(
                    $"Could not launch AutoGSE at \"{exePath}\" ({ex.Message}). " +
                    "Install AutoGSE, or set the AUTOGSE_EXE environment variable " +
                    "to its full autogse.exe path, then restart Playnite.", ex);
            }
        }

        private static string Quote(string path) => "\"" + path.Replace("\"", "\\\"") + "\"";

        /// <summary>
        /// Looks this game's InstallDirectory up in `autogse list --json`'s
        /// output (every target AutoGSE has ever injected on this machine) —
        /// there is no single-target "status of just this folder" CLI verb
        /// today, so this is the closest real data source, exactly the
        /// schema `engine::run_list`'s JSON branch emits.
        /// </summary>
        private static AutoGseJsonTarget FindKnownTarget(string installDirectory)
        {
            var (exitCode, stdout, _) = RunAutoGse("list --json");
            if (exitCode != 0)
            {
                return null;
            }

            List<AutoGseJsonTarget> targets;
            try
            {
                targets = JsonSerializer.Deserialize<List<AutoGseJsonTarget>>(stdout) ?? new List<AutoGseJsonTarget>();
            }
            catch (JsonException ex)
            {
                Logger.Error(ex, "Failed to parse `autogse list --json` output.");
                return null;
            }

            return targets.FirstOrDefault(t => IsUnderOrEqual(t.Path, installDirectory));
        }

        /// <summary>
        /// Real bug found live: this used to be an exact-path match, which
        /// silently failed for any game whose actual `steam_api(64).dll`
        /// sits below Playnite's own `InstallDirectory` (e.g. a nested
        /// `Binaries\Win64\` layout) — confirmed on a real game where "Check
        /// Status" reported "not known" while "Inject" (which independently
        /// re-runs discovery from the same root) correctly found it already
        /// injected. Both `engine::run_inject_single` and
        /// `run_revert_single` resolve their target via the same recursive
        /// `discovery::resolve_target(path, ...)` regardless of how deep the
        /// real DLL is nested beneath whatever `--path` is given — so a
        /// known target's recorded path is expected to be equal to, or
        /// nested under, `installDirectory`, not necessarily identical to it.
        /// </summary>
        private static bool IsUnderOrEqual(string targetPath, string installDirectory)
        {
            string target = System.IO.Path.GetFullPath(targetPath).TrimEnd('\\');
            string root = System.IO.Path.GetFullPath(installDirectory).TrimEnd('\\');
            return target.Equals(root, StringComparison.OrdinalIgnoreCase)
                || target.StartsWith(root + "\\", StringComparison.OrdinalIgnoreCase);
        }

        public override IEnumerable<GameMenuItem> GetGameMenuItems(GetGameMenuItemsArgs args)
        {
            var game = args.Games.FirstOrDefault();
            if (game == null || string.IsNullOrEmpty(game.InstallDirectory) || !Directory.Exists(game.InstallDirectory))
            {
                yield break;
            }

            // Real bug found live (The Binding of Isaac: Rebirth): passing
            // just the install folder made AutoGSE's own discovery pick a
            // DLL bundled with bundled helper tool deeper in the tree
            // instead of the real one beside the actual game exe — fixed
            // Rust-side (discovery.rs: a DLL beside a *specifically named*
            // exe now wins outright, matching real Windows DLL-search-order
            // semantics), but that fix only helps when a specific exe is
            // actually given. Resolving Playnite's own play action here
            // (rather than just the folder) lets this plugin take advantage
            // of it too, not just the Explorer "right-click the .exe"
            // context-menu path.
            string targetPath = ResolvePlayActionExePath(game) ?? game.InstallDirectory;

            yield return new GameMenuItem
            {
                Description = "AutoGSE: Check Status",
                MenuSection = "AutoGSE",
                Action = _ =>
                {
                    var target = FindKnownTarget(game.InstallDirectory);
                    string message = target == null
                        ? $"{game.Name}: not known to AutoGSE (vanilla, or never injected on this machine)."
                        : $"{game.Name}: {target.Status} (mode: {target.Mode ?? "n/a"}, AppID: {target.AppId?.ToString() ?? "n/a"}).";
                    PlayniteApi.Dialogs.ShowMessage(message, "AutoGSE Status");
                },
            };

            yield return new GameMenuItem
            {
                Description = "AutoGSE: Inject",
                MenuSection = "AutoGSE",
                Action = _ => InjectWithAppIdFallback(game, targetPath),
            };

            yield return new GameMenuItem
            {
                Description = "AutoGSE: Revert",
                MenuSection = "AutoGSE",
                Action = _ => RunAndReport("revert", targetPath, game.Name),
            };

            yield return new GameMenuItem
            {
                Description = "AutoGSE: Reinject (after a Steam update)",
                MenuSection = "AutoGSE",
                // Deliberately no --silent/--anon/--appid here: `reinject`
                // has its own narrow arg set (`--path`/`--json` only,
                // confirmed via src/cli.rs's ReinjectArgs) — it restages an
                // already-injected target's DLL from its existing manifest
                // and never touches App-ID resolution or Steam login at
                // all, so none of those flags exist for it or are needed.
                Action = _ => RunAndReport("reinject", targetPath, game.Name),
            };
        }

        /// <summary>
        /// Resolves the real play action's executable. Real bug found live,
        /// via diagnostic logging after this still didn't work the first
        /// time: `GameAction.Path` isn't a literal path fragment at all —
        /// Playnite stores it as a token string (confirmed against a real
        /// game: `Path=[{InstallDir}\isaac-ng.exe]`), so hand-combining it
        /// with `InstallDirectory` produced a nonexistent
        /// `...\{InstallDir}\isaac-ng.exe` path, silently failing `File.Exists`
        /// and falling back to the plain folder every time — the real
        /// exe-path fix below never actually fired through the plugin
        /// despite being fully correct on the Rust side. Fixed by using
        /// `IPlayniteAPI.ExpandGameVariables(Game, GameAction)` (confirmed
        /// via reflection against the real `Playnite.SDK.dll` to exist for
        /// exactly this purpose), which returns a new `GameAction` with
        /// every token — `{InstallDir}` and others — properly expanded,
        /// instead of hand-rolling token substitution.
        /// </summary>
        private string ResolvePlayActionExePath(Game game)
        {
            var playAction = game.GameActions?.FirstOrDefault(
                a => a.IsPlayAction && a.Type == GameActionType.File);
            if (playAction == null || string.IsNullOrEmpty(playAction.Path))
            {
                return null;
            }

            var expanded = PlayniteApi.ExpandGameVariables(game, playAction);
            return File.Exists(expanded.Path) ? expanded.Path : null;
        }

        /// <summary>
        /// `verb` is "inject", "revert", or "reinject". `extraArgs` is
        /// appended as-is (already includes its own leading space), used
        /// only by Inject to pass a reused/manually-supplied `--appid`.
        /// </summary>
        private void RunAndReport(string verb, string installDirectory, string gameName, string extraArgs = "")
        {
            var (exitCode, stdout, stderr) = RunAutoGseAction(verb, installDirectory, extraArgs);
            ReportResult(verb, gameName, exitCode, stdout, stderr);
        }

        /// <summary>
        /// --silent and --anon on inject/revert, always — real bug found
        /// live: without them, an inject that needs App-ID disambiguation
        /// (or, in principle, an authenticated run hitting a live Steam
        /// Guard prompt) tries to show an interactive prompt on a process
        /// with no real attached console, which can't work and fails
        /// outright rather than hanging — confirmed via the exact
        /// "prompt cancelled or non-interactive" failure this was built to
        /// fix. `--silent` makes that a clean, immediate, readable failure
        /// instead; `--anon` avoids the same category of risk on the
        /// Steam-login side. Same reasoning, and the same flag pair,
        /// Cheevos's own utils/autogse-bridge.js already applies to every
        /// inject/revert call it makes. `reinject` accepts neither flag
        /// (confirmed via its own narrower ReinjectArgs) so it's excluded.
        /// </summary>
        private static (int ExitCode, string Stdout, string Stderr) RunAutoGseAction(string verb, string installDirectory, string extraArgs)
        {
            string flags = verb == "reinject" ? "" : " --silent --anon";
            return RunAutoGse($"{verb} --path {Quote(installDirectory)}{flags}{extraArgs}");
        }

        private void ReportResult(string verb, string gameName, int exitCode, string stdout, string stderr)
        {
            string title = $"AutoGSE: {verb}";
            string body = exitCode == 0
                ? (string.IsNullOrWhiteSpace(stdout) ? $"{gameName}: {verb} succeeded." : stdout)
                : $"{gameName}: {verb} failed (exit {exitCode}).\n{stderr}";
            PlayniteApi.Dialogs.ShowMessage(body, title);
        }

        /// <summary>
        /// A plain-integer <c>Game.GameId</c> means this game is managed by
        /// Playnite's own Steam library plugin, which sets GameId to the
        /// real Steam App ID — a source that survives an AutoGSE revert
        /// untouched, since it's Playnite's own data, unlike AutoGSE's
        /// known-target index (`engine::run_revert_single` calls
        /// `index::forget`, so a reverted target has nothing left to reuse
        /// there). Real, confirmed-live exception: a game added to Playnite
        /// manually (no owning library plugin, `PluginId` the all-zero
        /// GUID) has no such hint at all — `GameId` is then just an opaque
        /// Playnite-internal identifier, not a Steam App ID, so parsing it
        /// as one would silently pass the wrong number. `FindKnownTarget`
        /// is still tried as a fallback (useful for a non-revert re-inject,
        /// e.g. over a `needs_update` target, where AutoGSE's own index
        /// entry was never erased).
        /// </summary>
        private static string KnownAppIdArgs(Game game)
        {
            if (ulong.TryParse(game.GameId, out var steamAppId))
            {
                return $" --appid {steamAppId}";
            }

            var known = FindKnownTarget(game.InstallDirectory);
            return known?.AppId != null ? $" --appid {known.AppId.Value}" : "";
        }

        /// <summary>
        /// Real gap found live: a game Playnite doesn't manage via its
        /// Steam library plugin (added manually — confirmed against a real
        /// case, `PluginId` the all-zero GUID) carries no Steam App ID
        /// hint anywhere accessible to this plugin. If AutoGSE's own
        /// automatic cascade also can't confidently resolve one (no
        /// `steam_appid.txt`, no PE metadata hint, no confident fuzzy
        /// match), `inject --silent` fails cleanly with exit 15
        /// (`AppIdResolutionFailed`, see CONTRACT.md's exit-code table) —
        /// which is correct behavior, not a bug, but leaving it at "failed"
        /// forces the user out to a terminal for something this plugin can
        /// just ask for directly. Prompts once via Playnite's own
        /// `SelectString` dialog (confirmed against the real restored
        /// `Playnite.SDK.dll` — `StringSelectionDialogResult { bool Result,
        /// string SelectedString }` — rather than guessed from memory) and
        /// retries with the supplied App ID if one was given and parses.
        /// </summary>
        private void InjectWithAppIdFallback(Game game, string targetPath)
        {
            string extraArgs = KnownAppIdArgs(game);
            var (exitCode, stdout, stderr) = RunAutoGseAction("inject", targetPath, extraArgs);

            if (exitCode == 15 && string.IsNullOrEmpty(extraArgs))
            {
                var prompt = PlayniteApi.Dialogs.SelectString(
                    $"AutoGSE couldn't automatically determine {game.Name}'s Steam App ID.\n\n" +
                    "Find it in the game's Steam store URL " +
                    "(store.steampowered.com/app/<ID>/...) or on steamdb.info, " +
                    "then enter it below.",
                    "AutoGSE: Steam App ID needed",
                    "");
                if (prompt.Result && ulong.TryParse(prompt.SelectedString, out var manualAppId))
                {
                    (exitCode, stdout, stderr) = RunAutoGseAction("inject", targetPath, $" --appid {manualAppId}");
                }
            }

            ReportResult("inject", game.Name, exitCode, stdout, stderr);
        }
    }
}
