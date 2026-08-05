# AutoGSE CLI JSON Contract

This document is the single source of truth for every `--json` output shape
AutoGSE's CLI produces, and the stable string tokens/exit codes companion
tooling (primarily the [Cheevos fork](../Cheevos), via
`utils/autogse-bridge.js`) can depend on. See
`roadmap-cheevos-integration.md` Phase 4 for why this exists: the goal is one
place to check instead of reading Rust source from the Node side.

**Scope of the guarantee**: every field and token listed here is stable
across releases per the versioning policy at the bottom of this document.
Anything AutoGSE prints that is *not* documented here (plain
human-readable text, log file contents, notification/toast text) is **not**
part of the contract and may change at any time without a version bump.

All JSON is emitted via `serde_json::to_string_pretty` — pretty-printed, one
top-level value per invocation, always the only thing written to stdout when
`--json` is passed (all `[AutoGSE] ...` info lines are suppressed; warnings
still go to stderr as plain text, uninvolved in the JSON contract).

Success/failure is carried entirely by the process exit code (see the table
below), **not** by a field inside the JSON body — a non-zero exit means the
JSON payload for that invocation was never printed at all (the process
failed before reaching its `--json` branch, or partway through). Callers
should check the exit code first.

---

## `inject --json` / `inject --root ... --json`

Single-target: one `JsonInjectResult` object. Batch (`--root`): one
`JsonInjectBatchResult` object wrapping one entry per target.

### `JsonInjectResult`

| Field | Type | Present when |
|---|---|---|
| `path` | string | always |
| `status` | string, one of: `"injected"`, `"already_injected"`, `"cancelled"`, `"dry_run"` | always |
| `dry_run` | bool | always |
| `mode` | string, `"regular"` \| `"steamclient"` | not for `already_injected` |
| `arch` | string, `"x86"` \| `"x64"` | not for `already_injected` |
| `app_id` | u64 | once App ID resolution has run (not for `already_injected`/`cancelled`) |
| `app_id_source` | string | same as `app_id` |
| `game_title` | string \| null | same as `app_id`, and only if a real title was resolved |
| `achievement_data_included` | bool | same as `app_id` |
| `achievement_data_note` | string | same as `app_id` |
| `generated_files` | string[] | `dry_run` status only — the file list a real run would write, including `steam_appid.txt` |

`status` meanings: `injected` — a real injection completed.
`already_injected` — no-op; target already has a manifest, run `revert`
first. `cancelled` — an interactive anti-cheat/anti-tamper prompt was
declined (never happens under `--silent`, which proceeds instead and only
warns to stderr). `dry_run` — `--dry-run` was passed; nothing on disk
changed.

### `JsonInjectBatchResult` (only when `--root` was used)

```
{ "results": [ <JsonInjectBatchEntry>, ... ], "succeeded": N, "failed": N, "total": N }
```

Each `JsonInjectBatchEntry` is internally tagged on `outcome`:
- `{"outcome":"ok", ...every JsonInjectResult field above}`
- `{"outcome":"error","path":"...","message":"..."}`

A batch run's own process exit code is always 0 if it got as far as running
(individual target failures are captured per-entry, not surfaced as the
overall exit code) — check `failed`/`results[].outcome` to detect partial
failure, not the exit code.

---

## `revert --json` / `revert --root ... --json`

Same single/batch split as `inject`.

### `JsonRevertResult`

| Field | Type | Present when |
|---|---|---|
| `path` | string | always |
| `status` | string, one of: `"reverted"`, `"nothing_to_revert"`, `"dry_run"` | always |
| `dry_run` | bool | always |
| `restored_file_count` | usize | `status: "reverted"` only |
| `removed_file_count` | usize | `status: "reverted"` only |
| `leftover_backup_folder_count` | usize | `status: "reverted"` only — count of `steam_settings.bak_*` folders left in place for manual review |
| `would_restore` | `JsonRevertRestorePreview[]` | `status: "dry_run"` only |
| `would_remove_files` | string[] | `status: "dry_run"` only |
| `would_remove_steam_settings` | bool | `status: "dry_run"` only |

`JsonRevertRestorePreview`: `{ "original_path", "backup_path", "ok" (bool — true iff the backup is present and hash-verified, i.e. a real revert would succeed here), "detail" }`.

### `JsonRevertBatchResult` (only when `--root` was used)

Same shape as `JsonInjectBatchResult`, with `JsonRevertResult` entries.

---

## `repair --json`

One `JsonRepairResult` object:

```
{ "path": "...", "diagnosis": "<token>", "action_taken": bool, "detail": "..." (optional) }
```

`diagnosis` tokens (from `RepairDiagnosis::as_json_str()`, shared with
`audit --json`'s `diagnosis` field below): `healthy`, `no_manifest`,
`orphaned_backup`, `backup_missing`, `backup_hash_mismatch`,
`stale_manifest_version`, `update_reverted`.

Only `healthy`, `update_reverted`, and `orphaned_backup` ever reach the JSON
body — the other four diagnoses are unrecoverable/require a fresh `inject`
and return a non-zero exit instead (see `RepairFailed`/`NotInjected` in the
exit code table). `action_taken` is `true` only for `orphaned_backup` (the
one diagnosis `repair` actually fixes automatically).

---

## `reinject --json`

One `JsonReinjectResult` object:

```
{ "path": "...", "arch": "x86" | "x64", "restaged_count": usize }
```

---

## `login --json`

```
{ "success": true, "username": "..." }
```

Never includes the password. On failure the process exits non-zero and no
JSON is printed (see `LoginFailed` in the exit code table).

## `logout --json`

```
{ "success": true }
```

---

## `doctor --json`

One `JsonDoctorReport` object:

```
{
  "tool_checks": [ { "name": "...", "ok": bool, "detail": "..." }, ... ],
  "dpapi_ok": bool,
  "dpapi_detail": "...",
  "known_target_count": usize | null,
  "logged_in_as": "..." | null
}
```

`tool_checks[].name` is always one of the four vendored tool names:
`generate_emu_config`, `parse_controller_vdf`, `lobby_connect`,
`steamclient_experimental`. `detail` is the resolved path on success or the
error message on failure. `logged_in_as` reflects credentials currently
stored on disk (never the password) — the only way to know login state after
a restart, since `login`/`logout --json` only ever report the outcome of the
action taken in that invocation. Deliberately omits the human log tail —
free-text console history, not something a caller should branch on.

---

## `scan --json` / `list --json`

Both emit a JSON array of `JsonTarget`:

```
{ "path": "...", "status": "<token>", "mode": "..." (optional), "app_id": u64 (optional), "game_title": "..." (optional) }
```

`status` tokens (`ScanStatus::as_json_str()`): `vanilla`, `injected`,
`needs_update`, `update_reverted`. `mode`/`app_id`/`game_title` are present
only for targets with a manifest (i.e. not `vanilla`). `scan` walks
`--root`'s immediate subfolders fresh every call; `list` reads AutoGSE's own
cross-machine known-target index instead (`known_target_count` in
`doctor --json` above is this same index's size).

---

## `audit --json`

A JSON array of `AuditFinding`:

```
{ "path": "...", "diagnosis": "<token>", "detail": "..." (optional, only present when the diagnosis is a real problem) }
```

Same `diagnosis` tokens as `repair --json` above (`RepairDiagnosis::as_json_str()`). Unlike `repair`, `audit` is read-only and reports every target under `--root`, including `healthy` ones — `detail` is only populated for problem diagnoses. A non-empty problem set causes the process to exit non-zero (`AuditFoundProblems`) even though the JSON body itself printed successfully — check the array contents, not just the exit code, if you need per-target detail before that happens.

---

## `rpcs3-trophies --path ... --json`

A JSON array of `TrophyWithState` (see `src/rpcs3.rs`):

```
{
  "trophy": { "id": u32, "name": "...", "description": "...", "hidden": bool, "grade": "<grade>", "platinum_link_id": u32 | null, "icon_path": "..." | null },
  "state": { "trophy_id": u32, "grade": "<grade>", "platinum_link_id": u32 | null, "unlocked": bool, "timestamp1": u64 | null, "timestamp2": u64 | null } | null
}
```

`grade` is one of `"bronze"`, `"silver"`, `"gold"`, `"platinum"`,
`"unknown"` (serde's default enum-variant naming, lowercased). `state` is
`null` when `TROPUSR.DAT` is missing entirely or has no entry yet for that
trophy (never launched / never unlocked anything).

---

## `export-achievements --format json`

A JSON array of `AchievementExportRow`:

```
{ "target_path": "...", "app_id": u64, "game_title": "..." | null, "achievement_name": "...", "display_name": "...", "unlocked": bool, "unlocked_at": u64 | null }
```

Note this command's flag is `--format json` (shared with `--format csv`), not
a standalone `--json` boolean like every other command above.

---

## Exit codes

Stable across releases (`AutoGseError::exit_code()`, `src/error.rs`) — a
script/bridge can branch on these without parsing stderr text. `0` is
success on every command. Selected codes most relevant to companion tooling:

| Code | Meaning |
|---|---|
| 2 | Target path does not exist |
| 3 | No `steam_api(64).dll` found under target |
| 4 | Target's DLL is locked by a running process (likely the game itself) |
| 5 | Another AutoGSE operation is already running against this target |
| 9 | Backup hash mismatch — refusing to revert |
| 15 | Could not determine a Steam App ID |
| 18 | Vendored GSE tools not found (see `doctor`) |
| 19 | Credential storage error |
| 20 | Steam login failed |
| 23 | Not an injected target (no `.gse_manifest.json`) |
| 33 | Already an injected target (`import --force` not passed) |
| 35 | `reinject` not applicable to this target's current state |
| 36 | `repair` diagnosed a problem it can't safely auto-fix |
| 37 | `audit` found one or more targets with an integrity problem |
| 1 | Any other error not covered by a dedicated code above |

Every other variant in `src/error.rs`'s `exit_code()` match is also stable
once assigned — this table lists the ones a Cheevos-style caller is most
likely to branch on, not the full 39-entry list; read `exit_code()` directly
for the complete mapping if a code not listed here comes up.

---

## Versioning policy

`autogse --version` prints the crate's real semver (from `Cargo.toml`) —
no separate "contract version" number exists. Compatibility rule while the
crate stays pre-1.0 (`0.MINOR.PATCH`):

- **Additive, non-breaking** (new optional field, new stable token appended
  to an existing enum-like field, a new `--json`-supporting command): no
  required version bump; may still land in a `PATCH` release.
- **Breaking** (a field removed/renamed, a token's meaning changed, a
  field's type changed): requires at least a `MINOR` bump. After 1.0 this
  becomes a `MAJOR` bump per normal semver.

A caller that wants forward-compatibility should ignore unknown JSON fields
(standard practice, and exactly what this policy is designed to make safe)
rather than validating an exact schema.

Asserting a minimum compatible AutoGSE version at startup (e.g. before the
first bridge call) is the *caller's* responsibility —
`utils/autogse-bridge.js` in the Cheevos repo, not something AutoGSE itself
enforces.
