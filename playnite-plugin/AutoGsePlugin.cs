using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;
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
        /// Resolved from the AUTOGSE_EXE environment variable if set,
        /// otherwise assumes "autogse.exe" is on PATH (true after a real
        /// AutoGSE install, since its installer registers Explorer
        /// integration but does not add PATH — documented limitation: a user
        /// installing only this plugin without a full AutoGSE install must
        /// set AUTOGSE_EXE themselves). No blind path-guessing against
        /// Program Files, since AutoGSE's own installer lets the user pick
        /// an arbitrary install directory (Phase 4 §4.2).
        /// </summary>
        private static string AutoGseExePath()
        {
            return Environment.GetEnvironmentVariable("AUTOGSE_EXE") ?? "autogse.exe";
        }

        private static (int ExitCode, string Stdout, string Stderr) RunAutoGse(string arguments)
        {
            var psi = new ProcessStartInfo(AutoGseExePath(), arguments)
            {
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
            };

            using (var proc = Process.Start(psi))
            {
                string stdout = proc.StandardOutput.ReadToEnd();
                string stderr = proc.StandardError.ReadToEnd();
                proc.WaitForExit();
                return (proc.ExitCode, stdout, stderr);
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

            return targets.FirstOrDefault(t =>
                string.Equals(
                    System.IO.Path.GetFullPath(t.Path).TrimEnd('\\'),
                    System.IO.Path.GetFullPath(installDirectory).TrimEnd('\\'),
                    StringComparison.OrdinalIgnoreCase));
        }

        public override IEnumerable<GameMenuItem> GetGameMenuItems(GetGameMenuItemsArgs args)
        {
            var game = args.Games.FirstOrDefault();
            if (game == null || string.IsNullOrEmpty(game.InstallDirectory) || !Directory.Exists(game.InstallDirectory))
            {
                yield break;
            }

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
                Action = _ => RunAndReport("inject", game.InstallDirectory, game.Name),
            };

            yield return new GameMenuItem
            {
                Description = "AutoGSE: Revert",
                MenuSection = "AutoGSE",
                Action = _ => RunAndReport("revert", game.InstallDirectory, game.Name),
            };

            yield return new GameMenuItem
            {
                Description = "AutoGSE: Reinject (after a Steam update)",
                MenuSection = "AutoGSE",
                Action = _ => RunAndReport("reinject", game.InstallDirectory, game.Name),
            };
        }

        private void RunAndReport(string verb, string installDirectory, string gameName)
        {
            var (exitCode, stdout, stderr) = RunAutoGse($"{verb} --path {Quote(installDirectory)}");
            string title = $"AutoGSE: {verb}";
            string body = exitCode == 0
                ? (string.IsNullOrWhiteSpace(stdout) ? $"{gameName}: {verb} succeeded." : stdout)
                : $"{gameName}: {verb} failed (exit {exitCode}).\n{stderr}";
            PlayniteApi.Dialogs.ShowMessage(body, title);
        }
    }
}
