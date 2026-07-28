//! Phase 7 §7.7's RetroAchievements.org integration: a thin client over the
//! real, official RetroAchievements Web API (endpoints/response shapes below
//! were confirmed live against `api-docs.retroachievements.org` and
//! `docs.retroachievements.org` while writing this module — not guessed from
//! training data, same discipline every other phase in this project applies).
//!
//! **Honest scope note, built with the user's explicit "scaffold now, verify
//! later" decision**: no real RetroAchievements account/API key is available
//! in this environment, so every network call here is exercised only by an
//! `#[ignore]`d live test (same convention as `steam_api`/`header_cache`) —
//! none has actually been run against the real API yet. Everything else
//! (request/response parsing, hash computation, credential storage, the
//! unlock-diff/notify logic) is unit tested against the *verified* schema.
//!
//! **Hash scanning is deliberately incomplete, not guessed**: RetroAchievements'
//! own developer docs (`docs.retroachievements.org/developer-docs/game-identification.html`)
//! confirm ROM identification is genuinely console-specific — most systems
//! hash the raw file, several strip a fixed header, and disc-based/complex
//! systems (PS1, PS2, PSP, Saturn, Dreamcast, GameCube, Nintendo DS, Neo Geo
//! CD, 3DO, Jaguar CD, PC-FX, Arcade) use custom boot-code/metadata hashing
//! that isn't documented anywhere this module's author could verify.
//! Implementing those from memory would repeat exactly the class of mistake
//! this project's roadmap has caught and corrected several times before
//! (e.g. the `img`/`images` folder-name guess in Phase 5) — so
//! [`hash_rule_for_console_name`] only covers the consoles whose rule is
//! concretely documented, and returns `None` (a clear "not supported yet"
//! signal) for everything else.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

use crate::credentials::{dpapi_protect, dpapi_unprotect, store_dir};
use crate::error::AutoGseError;
use crate::notify;

const API_BASE: &str = "https://retroachievements.org/API";
const BADGE_BASE: &str = "https://i.retroachievements.org/Badge";
const RA_CREDENTIALS_FILENAME: &str = "ra_credentials.dat";
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Credentials: a *separate* DPAPI-encrypted secret from Steam's
// `credentials.dat` (Phase 5) — a different service, a different secret, not
// crammed into the same struct/file just because the roadmap's original
// wording said "credentials.dat" singular. Reuses Phase 5's DPAPI wrapper
// functions directly rather than re-implementing the same unsafe FFI twice.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct RaCredentials {
    pub username: String,
    pub api_key: String,
}

fn ra_credentials_path(dir: &Path) -> PathBuf {
    dir.join(RA_CREDENTIALS_FILENAME)
}

pub fn save_in(dir: &Path, creds: &RaCredentials) -> Result<(), AutoGseError> {
    std::fs::create_dir_all(dir)?;
    let mut plaintext = serde_json::to_vec(creds)?;
    let encrypted = dpapi_protect(&mut plaintext)?;
    std::fs::write(ra_credentials_path(dir), encrypted)?;
    Ok(())
}

pub fn load_in(dir: &Path) -> Result<Option<RaCredentials>, AutoGseError> {
    let path = ra_credentials_path(dir);
    if !path.is_file() {
        return Ok(None);
    }
    let mut encrypted = std::fs::read(&path)?;
    let decrypted = dpapi_unprotect(&mut encrypted)?;
    Ok(Some(serde_json::from_slice(&decrypted)?))
}

pub fn delete_in(dir: &Path) -> Result<(), AutoGseError> {
    let path = ra_credentials_path(dir);
    if path.is_file() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn save(creds: &RaCredentials) -> Result<(), AutoGseError> {
    save_in(&store_dir()?, creds)
}

pub fn load() -> Result<Option<RaCredentials>, AutoGseError> {
    load_in(&store_dir()?)
}

pub fn delete() -> Result<(), AutoGseError> {
    delete_in(&store_dir()?)
}

// ---------------------------------------------------------------------------
// API client: `GetGameInfoAndUserProgress` (game + achievement list + one
// user's unlock state) and `GetGameList` (per-console game+hash catalog,
// `h=1` — the real mechanism RA-integrated tools use to match a local ROM
// hash to a game; there is no documented single hash->GameID endpoint).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct RetroAchievement {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub points: u32,
    pub badge_name: String,
    pub unlocked: bool,
    pub unlocked_hardcore: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GameProgress {
    pub game_id: u64,
    pub title: String,
    pub console_name: String,
    pub num_achievements: u32,
    pub num_awarded_to_user: u32,
    pub achievements: Vec<RetroAchievement>,
}

#[derive(Debug, Deserialize)]
struct RawAchievementEntry {
    #[serde(rename = "ID")]
    id: u64,
    #[serde(rename = "Title")]
    title: String,
    #[serde(rename = "Description", default)]
    description: String,
    #[serde(rename = "Points", default)]
    points: u32,
    #[serde(rename = "BadgeName", default)]
    badge_name: String,
    // Present only when the queried user has earned this achievement —
    // confirmed live against the real API docs, not assumed; presence
    // (not its actual date value, which this module doesn't surface) is
    // what `unlocked`/`unlocked_hardcore` are derived from.
    #[serde(rename = "DateEarned")]
    date_earned: Option<String>,
    #[serde(rename = "DateEarnedHardcore")]
    date_earned_hardcore: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawGameInfoAndUserProgress {
    #[serde(rename = "ID")]
    id: u64,
    #[serde(rename = "Title")]
    title: String,
    #[serde(rename = "ConsoleName", default)]
    console_name: String,
    #[serde(rename = "NumAchievements", default)]
    num_achievements: u32,
    #[serde(rename = "NumAwardedToUser", default)]
    num_awarded_to_user: u32,
    #[serde(rename = "Achievements", default)]
    achievements: HashMap<String, RawAchievementEntry>,
}

fn build_agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(timeout))
        .timeout_global(Some(timeout))
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build())
        .build();
    ureq::Agent::new_with_config(config)
}

fn parse_game_progress_response(bytes: &[u8]) -> Result<GameProgress, AutoGseError> {
    let raw: RawGameInfoAndUserProgress = serde_json::from_slice(bytes)?;
    let mut achievements: Vec<RetroAchievement> = raw
        .achievements
        .into_values()
        .map(|a| RetroAchievement {
            id: a.id,
            title: a.title,
            description: a.description,
            points: a.points,
            badge_name: a.badge_name,
            unlocked: a.date_earned.is_some(),
            unlocked_hardcore: a.date_earned_hardcore.is_some(),
        })
        .collect();
    achievements.sort_by_key(|a| a.id);

    Ok(GameProgress {
        game_id: raw.id,
        title: raw.title,
        console_name: raw.console_name,
        num_achievements: raw.num_achievements,
        num_awarded_to_user: raw.num_awarded_to_user,
        achievements,
    })
}

/// Fetches one game's full achievement list plus `creds.username`'s unlock
/// progress against it. `GET API_GetGameInfoAndUserProgress.php?y=&u=&g=`
/// (confirmed live against the official API docs).
pub fn fetch_game_progress(creds: &RaCredentials, game_id: u64, timeout: Duration) -> Result<GameProgress, AutoGseError> {
    let agent = build_agent(timeout);
    let mut response = agent
        .get(format!("{API_BASE}/API_GetGameInfoAndUserProgress.php"))
        .query("y", &creds.api_key)
        .query("u", &creds.username)
        .query("g", game_id.to_string())
        .call()
        .map_err(|e| AutoGseError::RetroAchievements(format!("GetGameInfoAndUserProgress failed: {e}")))?;

    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|e| AutoGseError::RetroAchievements(format!("GetGameInfoAndUserProgress response read failed: {e}")))?;

    parse_game_progress_response(&bytes)
}

#[derive(Debug, Clone, PartialEq)]
pub struct GameListEntry {
    pub game_id: u64,
    pub title: String,
    pub hashes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawGameListEntry {
    #[serde(rename = "ID")]
    id: u64,
    #[serde(rename = "Title")]
    title: String,
    #[serde(rename = "Hashes", default)]
    hashes: Vec<String>,
}

fn parse_game_list_response(bytes: &[u8]) -> Result<Vec<GameListEntry>, AutoGseError> {
    let raw: Vec<RawGameListEntry> = serde_json::from_slice(bytes)?;
    Ok(raw.into_iter().map(|g| GameListEntry { game_id: g.id, title: g.title, hashes: g.hashes }).collect())
}

/// `GET API_GetGameList.php?y=&i=<console_id>&h=1` — every game for one RA
/// console ID, each with its full list of accepted ROM hashes. The real
/// mechanism RA-integrated tools use to resolve "which game is this hash,"
/// since no documented endpoint goes hash->GameID directly.
pub fn fetch_game_list_with_hashes(creds: &RaCredentials, console_id: u64, timeout: Duration) -> Result<Vec<GameListEntry>, AutoGseError> {
    let agent = build_agent(timeout);
    let mut response = agent
        .get(format!("{API_BASE}/API_GetGameList.php"))
        .query("y", &creds.api_key)
        .query("i", console_id.to_string())
        .query("h", "1")
        .call()
        .map_err(|e| AutoGseError::RetroAchievements(format!("GetGameList failed: {e}")))?;

    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|e| AutoGseError::RetroAchievements(format!("GetGameList response read failed: {e}")))?;

    parse_game_list_response(&bytes)
}

/// Pure local matching: which game (if any) in `games` lists `hash` among
/// its accepted hashes. Case-insensitive — RA's own hash strings and a
/// locally computed MD5 hex string aren't guaranteed to agree on case.
pub fn find_game_id_by_hash(games: &[GameListEntry], hash: &str) -> Option<u64> {
    games.iter().find(|g| g.hashes.iter().any(|h| h.eq_ignore_ascii_case(hash))).map(|g| g.game_id)
}

/// `https://i.retroachievements.org/Badge/<badge_name>.png` — confirmed live
/// against real RA badge URLs. No locked/grayscale variant is included here:
/// unlike Goldberg's `icon`/`icongray` pair (Phase 7 §7.5), no documented or
/// observed URL convention for a locked-state badge image was found while
/// researching this, so this deliberately isn't guessed at.
pub fn badge_url(badge_name: &str) -> String {
    format!("{BADGE_BASE}/{badge_name}.png")
}

/// Same disk-cache convention as `header_cache::cached_header_path` (Phase 7
/// §7.2): fetched once per badge name and reused, temp-sibling + rename for
/// atomicity.
pub fn cached_badge_path(badge_name: &str, timeout: Duration) -> Result<PathBuf, AutoGseError> {
    let dir = store_dir()?.join("ra_badge_cache");
    let path = dir.join(format!("{badge_name}.png"));
    if path.is_file() {
        return Ok(path);
    }

    let url = badge_url(badge_name);
    let agent = build_agent(timeout);
    let mut response = agent.get(&url).call().map_err(|e| AutoGseError::RetroAchievements(format!("badge fetch {url}: {e}")))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|e| AutoGseError::RetroAchievements(format!("badge fetch {url}: {e}")))?;

    std::fs::create_dir_all(&dir)?;
    let tmp_path = dir.join(format!("{badge_name}.png.tmp"));
    std::fs::write(&tmp_path, &bytes)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// ROM hash computation — see this module's own top doc comment for why only
// a documented subset of consoles is covered here.
// ---------------------------------------------------------------------------

/// How to derive a console's identifying hash from a ROM file, per
/// `docs.retroachievements.org/developer-docs/game-identification.html`.
/// Only the concretely documented, file-header-based rules are modeled —
/// disc-based/custom boot-code hashing is out of scope (see module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashRule {
    /// Plain MD5 of the entire file — the documented default for most
    /// non-disc systems not listed below.
    PlainMd5,
    /// Skip the first `strip_bytes` bytes if the file begins with `magic`;
    /// hash the rest. Covers NES (`NES\x1a`, 16 bytes), FDS (`FDS\x1a`, 16
    /// bytes), Atari 7800 (`\x01ATARI7800`, 128 bytes), and Atari Lynx
    /// (`LYNX\0`, 64 bytes).
    StripHeaderIfMagic { magic: &'static [u8], strip_bytes: usize },
    /// SNES: skip the first 512 bytes only when the file is 512 bytes larger
    /// than an exact multiple of 8KB (a copier header, not part of the ROM).
    Snes512HeaderIfOversized,
}

/// Maps a console name (as returned by RA's own `ConsoleName` field, e.g.
/// from [`GameProgress::console_name`]) to its documented hash rule. Returns
/// `None` for every console whose rule wasn't concretely documented anywhere
/// this module's author could verify — callers must treat that as "hash
/// identification not supported for this console yet," not fall back to a
/// guess.
pub fn hash_rule_for_console_name(console_name: &str) -> Option<HashRule> {
    match console_name {
        "NES/Famicom" | "NES" | "Famicom" => Some(HashRule::StripHeaderIfMagic { magic: b"NES\x1a", strip_bytes: 16 }),
        "Famicom Disk System" => Some(HashRule::StripHeaderIfMagic { magic: b"FDS\x1a", strip_bytes: 16 }),
        "Atari 7800" => Some(HashRule::StripHeaderIfMagic { magic: b"\x01ATARI7800", strip_bytes: 128 }),
        "Atari Lynx" => Some(HashRule::StripHeaderIfMagic { magic: b"LYNX\0", strip_bytes: 64 }),
        "SNES/Super Famicom" | "SNES" | "Super Famicom" => Some(HashRule::Snes512HeaderIfOversized),
        _ => None,
    }
}

fn md5_hex(bytes: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Computes `path`'s RetroAchievements-identifying hash per `rule`. Reads
/// the whole file into memory — real ROM files for the consoles this
/// supports are small enough (cartridge-era systems, tens of KB to a few MB)
/// that streaming isn't worth the complexity `Snes512HeaderIfOversized`'s
/// size-based branch would otherwise need.
pub fn compute_rom_hash(path: &Path, rule: HashRule) -> Result<String, AutoGseError> {
    let mut file = BufReader::new(File::open(path)?);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(md5_hex(&hash_input_for_rule(&bytes, rule)))
}

fn hash_input_for_rule(bytes: &[u8], rule: HashRule) -> &[u8] {
    match rule {
        HashRule::PlainMd5 => bytes,
        HashRule::StripHeaderIfMagic { magic, strip_bytes } => {
            if bytes.starts_with(magic) && bytes.len() > strip_bytes {
                &bytes[strip_bytes..]
            } else {
                bytes
            }
        }
        HashRule::Snes512HeaderIfOversized => {
            const HEADER: usize = 512;
            const BLOCK: usize = 8 * 1024;
            if bytes.len() > HEADER && (bytes.len() - HEADER) % BLOCK == 0 {
                &bytes[HEADER..]
            } else {
                bytes
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unlock tracking / toast hook (roadmap: "Retro achievement unlock tracker
// and toast notification hook mirroring Steam overlay achievement popups").
// RA achievements unlock server-side while playing in an RA-integrated
// emulator, not something AutoGSE itself triggers — so "tracking" here means
// periodically re-fetching a game's progress and diffing against what was
// last seen, the same shape as polling rather than a live local event.
// ---------------------------------------------------------------------------

fn seen_unlocks_path(game_id: u64) -> Result<PathBuf, AutoGseError> {
    Ok(store_dir()?.join("ra_seen_unlocks").join(format!("{game_id}.json")))
}

pub fn load_seen_unlocks(game_id: u64) -> Result<HashSet<u64>, AutoGseError> {
    let path = seen_unlocks_path(game_id)?;
    if !path.is_file() {
        return Ok(HashSet::new());
    }
    Ok(serde_json::from_slice(&std::fs::read(&path)?)?)
}

pub fn save_seen_unlocks(game_id: u64, seen: &HashSet<u64>) -> Result<(), AutoGseError> {
    let path = seen_unlocks_path(game_id)?;
    std::fs::create_dir_all(path.parent().expect("seen_unlocks_path always has a parent"))?;
    std::fs::write(&path, serde_json::to_vec(seen)?)?;
    Ok(())
}

/// Pure diff: which of `achievements` are unlocked but weren't yet recorded
/// in `previously_seen`. Order matches `achievements`' own (ID-ascending,
/// per `parse_game_progress_response`).
pub fn detect_new_unlocks<'a>(previously_seen: &HashSet<u64>, achievements: &'a [RetroAchievement]) -> Vec<&'a RetroAchievement> {
    achievements.iter().filter(|a| a.unlocked && !previously_seen.contains(&a.id)).collect()
}

/// Fetches current progress, diffs against the local "already notified"
/// cache, fires one `notify::show` toast per newly-unlocked achievement
/// (mirroring `engine::run_inject_single`'s Steam achievement-complete
/// toast), then persists the updated seen-set. Returns the newly-unlocked
/// list too, for a GUI caller that wants to render it directly rather than
/// re-deriving it from a toast.
pub fn check_for_new_unlocks(creds: &RaCredentials, game_id: u64, timeout: Duration) -> Result<Vec<RetroAchievement>, AutoGseError> {
    let progress = fetch_game_progress(creds, game_id, timeout)?;
    let mut seen = load_seen_unlocks(game_id)?;

    let new_unlocks: Vec<RetroAchievement> = detect_new_unlocks(&seen, &progress.achievements).into_iter().cloned().collect();
    for achievement in &new_unlocks {
        notify::show("AutoGSE: RetroAchievements Unlocked", &format!("{} — {}", progress.title, achievement.title));
        seen.insert(achievement.id);
    }
    if !new_unlocks.is_empty() {
        save_seen_unlocks(game_id, &seen)?;
    }

    Ok(new_unlocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_creds() -> RaCredentials {
        RaCredentials { username: "retro_user".to_string(), api_key: "abc123secret".to_string() }
    }

    #[test]
    fn ra_credentials_round_trip_through_save_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_in(dir.path()).unwrap().is_none());

        save_in(dir.path(), &sample_creds()).unwrap();
        assert_eq!(load_in(dir.path()).unwrap().unwrap(), sample_creds());

        delete_in(dir.path()).unwrap();
        assert!(load_in(dir.path()).unwrap().is_none());
    }

    #[test]
    fn ra_credentials_stored_file_is_not_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        save_in(dir.path(), &sample_creds()).unwrap();
        let on_disk = String::from_utf8_lossy(&std::fs::read(ra_credentials_path(dir.path())).unwrap()).into_owned();
        assert!(!on_disk.contains("abc123secret"));
        assert!(!on_disk.contains("retro_user"));
    }

    /// Built from the real, verified `GetGameInfoAndUserProgress` response
    /// schema (fetched live from `api-docs.retroachievements.org` while
    /// writing this module) — not a guessed shape. Three achievements:
    /// hardcore-earned, softcore-only-earned, and never earned.
    const SAMPLE_PROGRESS_JSON: &str = r#"{
        "ID": 14402,
        "Title": "Sample Game",
        "ConsoleID": 7,
        "ConsoleName": "NES/Famicom",
        "NumAchievements": 3,
        "NumAwardedToUser": 2,
        "NumAwardedToUserHardcore": 1,
        "Achievements": {
            "111": {"ID": 111, "Title": "First Steps", "Description": "Start the game", "Points": 5, "BadgeName": "012345", "DateEarned": "2024-01-01 12:00:00", "DateEarnedHardcore": "2024-01-01 12:00:00"},
            "112": {"ID": 112, "Title": "Softcore Only", "Description": "Earned without hardcore", "Points": 10, "BadgeName": "012346", "DateEarned": "2024-01-02 12:00:00"},
            "113": {"ID": 113, "Title": "Not Yet", "Description": "Never earned", "Points": 25, "BadgeName": "012347"}
        }
    }"#;

    #[test]
    fn parse_game_progress_response_reads_real_shape() {
        let progress = parse_game_progress_response(SAMPLE_PROGRESS_JSON.as_bytes()).unwrap();
        assert_eq!(progress.game_id, 14402);
        assert_eq!(progress.title, "Sample Game");
        assert_eq!(progress.console_name, "NES/Famicom");
        assert_eq!(progress.num_achievements, 3);
        assert_eq!(progress.achievements.len(), 3);
        assert_eq!(progress.achievements.iter().map(|a| a.id).collect::<Vec<_>>(), vec![111, 112, 113]);
    }

    #[test]
    fn parse_game_progress_response_derives_unlocked_from_date_earned_presence() {
        let progress = parse_game_progress_response(SAMPLE_PROGRESS_JSON.as_bytes()).unwrap();
        let by_id = |id: u64| progress.achievements.iter().find(|a| a.id == id).unwrap();

        assert!(by_id(111).unlocked);
        assert!(by_id(111).unlocked_hardcore);

        assert!(by_id(112).unlocked);
        assert!(!by_id(112).unlocked_hardcore);

        assert!(!by_id(113).unlocked);
        assert!(!by_id(113).unlocked_hardcore);
    }

    #[test]
    fn parse_game_progress_response_reads_points_and_badge_name() {
        let progress = parse_game_progress_response(SAMPLE_PROGRESS_JSON.as_bytes()).unwrap();
        let first = progress.achievements.iter().find(|a| a.id == 111).unwrap();
        assert_eq!(first.points, 5);
        assert_eq!(first.badge_name, "012345");
    }

    const SAMPLE_GAME_LIST_JSON: &str = r#"[
        {"Title": "Game A", "ID": 1, "ConsoleID": 7, "ConsoleName": "NES/Famicom", "Hashes": ["AAAA1111", "BBBB2222"]},
        {"Title": "Game B", "ID": 2, "ConsoleID": 7, "ConsoleName": "NES/Famicom", "Hashes": ["CCCC3333"]}
    ]"#;

    #[test]
    fn parse_game_list_response_reads_hashes() {
        let games = parse_game_list_response(SAMPLE_GAME_LIST_JSON.as_bytes()).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].game_id, 1);
        assert_eq!(games[0].hashes, vec!["AAAA1111".to_string(), "BBBB2222".to_string()]);
    }

    #[test]
    fn find_game_id_by_hash_matches_case_insensitively() {
        let games = parse_game_list_response(SAMPLE_GAME_LIST_JSON.as_bytes()).unwrap();
        assert_eq!(find_game_id_by_hash(&games, "cccc3333"), Some(2));
        assert_eq!(find_game_id_by_hash(&games, "bbbb2222"), Some(1));
        assert_eq!(find_game_id_by_hash(&games, "deadbeef"), None);
    }

    #[test]
    fn badge_url_matches_real_cdn_pattern() {
        assert_eq!(badge_url("012345"), "https://i.retroachievements.org/Badge/012345.png");
    }

    #[test]
    fn hash_rule_for_console_name_covers_documented_consoles() {
        assert_eq!(hash_rule_for_console_name("NES/Famicom"), Some(HashRule::StripHeaderIfMagic { magic: b"NES\x1a", strip_bytes: 16 }));
        assert_eq!(hash_rule_for_console_name("SNES/Super Famicom"), Some(HashRule::Snes512HeaderIfOversized));
        assert_eq!(hash_rule_for_console_name("Atari 7800"), Some(HashRule::StripHeaderIfMagic { magic: b"\x01ATARI7800", strip_bytes: 128 }));
    }

    #[test]
    fn hash_rule_for_console_name_is_none_for_undocumented_disc_based_systems() {
        // PlayStation's real RA hash uses custom boot-code/serial extraction,
        // not a documented plain-file or fixed-header rule — must not guess.
        assert_eq!(hash_rule_for_console_name("PlayStation"), None);
        assert_eq!(hash_rule_for_console_name("Nintendo GameCube"), None);
        assert_eq!(hash_rule_for_console_name("Arcade"), None);
    }

    #[test]
    fn compute_rom_hash_plain_md5_matches_known_vector() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rom.bin");
        std::fs::write(&path, b"hello world").unwrap();
        // Known MD5("hello world") test vector.
        assert_eq!(compute_rom_hash(&path, HashRule::PlainMd5).unwrap(), "5eb63bbbe01eeed093cb22bb8f5acdc3");
    }

    #[test]
    fn compute_rom_hash_strips_documented_nes_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rom.nes");
        let mut content = b"NES\x1a".to_vec();
        content.extend_from_slice(&[0u8; 12]); // rest of the 16-byte iNES header
        content.extend_from_slice(b"hello world");
        std::fs::write(&path, &content).unwrap();

        let rule = hash_rule_for_console_name("NES/Famicom").unwrap();
        // Hash of the header-stripped content must equal the plain hash of
        // just the ROM payload that followed the header.
        assert_eq!(compute_rom_hash(&path, rule).unwrap(), "5eb63bbbe01eeed093cb22bb8f5acdc3");
    }

    #[test]
    fn compute_rom_hash_leaves_file_untouched_when_magic_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rom.nes");
        std::fs::write(&path, b"hello world").unwrap(); // no NES magic at all

        let rule = hash_rule_for_console_name("NES/Famicom").unwrap();
        // No header to strip, so this must hash the whole (unmodified) file
        // -- i.e. identical to PlainMd5 on the same bytes.
        assert_eq!(compute_rom_hash(&path, rule).unwrap(), compute_rom_hash(&path, HashRule::PlainMd5).unwrap());
    }

    #[test]
    fn compute_rom_hash_snes_strips_512_byte_copier_header_only_when_oversized() {
        let dir = tempfile::tempdir().unwrap();

        // Exact 8KB multiple: no header to strip.
        let exact_path = dir.path().join("exact.smc");
        std::fs::write(&exact_path, vec![0xABu8; 8 * 1024]).unwrap();
        let rule = HashRule::Snes512HeaderIfOversized;
        assert_eq!(compute_rom_hash(&exact_path, rule).unwrap(), compute_rom_hash(&exact_path, HashRule::PlainMd5).unwrap());

        // 8KB multiple + 512: header must be stripped before hashing.
        let oversized_path = dir.path().join("oversized.smc");
        let mut oversized = vec![0xFFu8; 512]; // the copier header itself
        oversized.extend_from_slice(&[0xABu8; 8 * 1024]);
        std::fs::write(&oversized_path, &oversized).unwrap();
        assert_eq!(compute_rom_hash(&oversized_path, rule).unwrap(), compute_rom_hash(&exact_path, HashRule::PlainMd5).unwrap());
    }

    #[test]
    fn detect_new_unlocks_returns_only_unseen_unlocked_achievements() {
        let progress = parse_game_progress_response(SAMPLE_PROGRESS_JSON.as_bytes()).unwrap();
        let mut seen = HashSet::new();
        seen.insert(111u64); // already notified about this one

        let new_unlocks = detect_new_unlocks(&seen, &progress.achievements);
        assert_eq!(new_unlocks.len(), 1);
        assert_eq!(new_unlocks[0].id, 112); // unlocked, not yet seen
        // 113 is not unlocked at all, so it must never appear regardless of `seen`.
    }

    #[test]
    fn detect_new_unlocks_is_empty_when_everything_already_seen() {
        let progress = parse_game_progress_response(SAMPLE_PROGRESS_JSON.as_bytes()).unwrap();
        let seen: HashSet<u64> = progress.achievements.iter().filter(|a| a.unlocked).map(|a| a.id).collect();
        assert!(detect_new_unlocks(&seen, &progress.achievements).is_empty());
    }

    #[test]
    fn seen_unlocks_round_trips_through_save_load() {
        // Uses the real store_dir() (LOCALAPPDATA-based, like every other
        // local-state module in this codebase) with a throwaway game ID
        // unlikely to collide with anything real, then cleans up after itself.
        let game_id = 999_999_001u64;
        let mut seen = HashSet::new();
        seen.insert(1u64);
        seen.insert(2u64);

        save_seen_unlocks(game_id, &seen).unwrap();
        assert_eq!(load_seen_unlocks(game_id).unwrap(), seen);

        std::fs::remove_file(seen_unlocks_path(game_id).unwrap()).unwrap();
    }

    #[test]
    fn load_seen_unlocks_of_unknown_game_is_empty_not_an_error() {
        assert!(load_seen_unlocks(999_999_002).unwrap().is_empty());
    }

    /// Manual QA only (live network call against a real RA account, not run
    /// in normal `cargo test`) — no RetroAchievements API key is available
    /// in this environment, so this has never actually been run:
    /// `cargo test retroachievements::tests::live_fetch_game_progress -- --ignored`
    #[test]
    #[ignore]
    fn live_fetch_game_progress() {
        let creds = load().unwrap().expect("run `autogse ra-login` first");
        // Sonic the Hedgehog (Genesis) — a well-known, stable RA game ID,
        // same "pick something guaranteed to exist" convention as this
        // codebase's Cyberpunk 2077/Spacewar Steam AppID examples.
        let progress = fetch_game_progress(&creds, 1, Duration::from_millis(3000)).expect("live RA fetch");
        assert!(!progress.achievements.is_empty());
    }
}
