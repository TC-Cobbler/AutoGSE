//! Phase 9 §9.1's real, verified LAN/network helpers. Confirmed against the
//! vendored GSE fork's own README and `configs.main.EXAMPLE.ini`: there is no
//! `listen_port.txt`/`connect_ip.txt` and no "room code" concept anywhere in
//! real GSE — the only standalone network file is `custom_broadcasts.txt`
//! (one IP/domain per line, per-game at `steam_settings/custom_broadcasts.txt`
//! taking priority over a global copy), and `listen_port` is an INI key inside
//! `configs.main.ini`'s `[main::connectivity]` section (default `47584`), not
//! a file. Joining across a router/VPN (i.e. outside the local UDP broadcast
//! domain) means adding that peer's IP/domain here so the emulator sends
//! broadcast traffic there directly — `goldberg::run_lobby_connect` (§6.7)
//! remains the actual discovery/join UI, unchanged by this module.

use std::path::{Path, PathBuf};

use crate::error::AutoGseError;
use crate::ini_patch;

const BROADCASTS_FILENAME: &str = "custom_broadcasts.txt";
const CONNECTIVITY_SECTION: &str = "main::connectivity";
const LISTEN_PORT_KEY: &str = "listen_port";

/// The vendored fork's own documented default (`configs.main.EXAMPLE.ini`).
pub const DEFAULT_LISTEN_PORT: u16 = 47584;

fn configs_main_ini(tod: &Path) -> PathBuf {
    tod.join("steam_settings").join("configs.main.ini")
}

pub fn broadcasts_path(tod: &Path) -> PathBuf {
    tod.join("steam_settings").join(BROADCASTS_FILENAME)
}

/// Every non-blank line currently in this target's `custom_broadcasts.txt`,
/// in file order. Empty (not an error) when the file doesn't exist yet — a
/// freshly-injected target has no custom peers by default.
pub fn list_peers(tod: &Path) -> Result<Vec<String>, AutoGseError> {
    let path = broadcasts_path(tod);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(content.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

fn write_peers(tod: &Path, peers: &[String]) -> Result<(), AutoGseError> {
    let path = broadcasts_path(tod);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = if peers.is_empty() { String::new() } else { peers.join("\r\n") + "\r\n" };
    std::fs::write(&path, content)?;
    Ok(())
}

/// Adds `ip_or_domain` to this target's broadcast peer list. A no-op if
/// already present (this file has no other structure to disturb, unlike an
/// INI file, so there's nothing "existing" worth backing up before a plain
/// append).
pub fn add_peer(tod: &Path, ip_or_domain: &str) -> Result<(), AutoGseError> {
    let mut peers = list_peers(tod)?;
    if peers.iter().any(|p| p == ip_or_domain) {
        return Ok(());
    }
    peers.push(ip_or_domain.to_string());
    write_peers(tod, &peers)
}

/// Removes `ip_or_domain` from the broadcast peer list, if present.
pub fn remove_peer(tod: &Path, ip_or_domain: &str) -> Result<(), AutoGseError> {
    let peers: Vec<String> = list_peers(tod)?.into_iter().filter(|p| p != ip_or_domain).collect();
    write_peers(tod, &peers)
}

/// The emulator's own default (`47584`) when `configs.main.ini` doesn't set
/// `listen_port` explicitly, or doesn't exist yet.
pub fn get_listen_port(tod: &Path) -> Result<u16, AutoGseError> {
    let path = configs_main_ini(tod);
    if !path.is_file() {
        return Ok(DEFAULT_LISTEN_PORT);
    }
    let sections = ini_patch::read_all(&path)?;
    let value = sections
        .iter()
        .find(|s| s.name == CONNECTIVITY_SECTION)
        .and_then(|s| s.entries.iter().find(|e| e.key == LISTEN_PORT_KEY))
        .map(|e| e.value.clone());
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_LISTEN_PORT))
}

pub fn set_listen_port(tod: &Path, port: u16) -> Result<(), AutoGseError> {
    ini_patch::set_key(&configs_main_ini(tod), CONNECTIVITY_SECTION, LISTEN_PORT_KEY, &port.to_string())
}

/// Only two real, independent configurations — not the fictional "LAN
/// gaming/split-screen/custom port" modes roadmap §9.1 originally speced.
/// The vendored README explicitly warns against same-appid same-machine
/// multi-instance play, so no "split-screen" preset exists; cross-network
/// peer reachability is handled by [`add_peer`] directly rather than folded
/// into a preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPreset {
    /// Restores the emulator's own documented default port.
    Default,
    /// Every peer must agree on the same non-default port — useful when the
    /// default `47584` is blocked by a router/ISP.
    CustomPort(u16),
}

pub fn apply_preset(tod: &Path, preset: NetworkPreset) -> Result<(), AutoGseError> {
    match preset {
        NetworkPreset::Default => set_listen_port(tod, DEFAULT_LISTEN_PORT),
        NetworkPreset::CustomPort(port) => set_listen_port(tod, port),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_configs_main_ini(tod: &Path) {
        let settings = tod.join("steam_settings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("configs.main.ini"),
            "[main::general]\r\nnew_app_ticket=1\r\n\r\n[main::connectivity]\r\noffline=0\r\nlisten_port=47584\r\n",
        )
        .unwrap();
    }

    #[test]
    fn list_peers_is_empty_when_file_missing() {
        let tod = tempfile::tempdir().unwrap();
        assert!(list_peers(tod.path()).unwrap().is_empty());
    }

    #[test]
    fn add_peer_then_list_round_trips() {
        let tod = tempfile::tempdir().unwrap();
        add_peer(tod.path(), "10.0.0.5").unwrap();
        add_peer(tod.path(), "friend.duckdns.org").unwrap();

        assert_eq!(list_peers(tod.path()).unwrap(), vec!["10.0.0.5".to_string(), "friend.duckdns.org".to_string()]);
    }

    #[test]
    fn add_peer_is_idempotent() {
        let tod = tempfile::tempdir().unwrap();
        add_peer(tod.path(), "10.0.0.5").unwrap();
        add_peer(tod.path(), "10.0.0.5").unwrap();

        assert_eq!(list_peers(tod.path()).unwrap(), vec!["10.0.0.5".to_string()]);
    }

    #[test]
    fn remove_peer_deletes_only_the_matching_line() {
        let tod = tempfile::tempdir().unwrap();
        add_peer(tod.path(), "10.0.0.5").unwrap();
        add_peer(tod.path(), "10.0.0.6").unwrap();

        remove_peer(tod.path(), "10.0.0.5").unwrap();

        assert_eq!(list_peers(tod.path()).unwrap(), vec!["10.0.0.6".to_string()]);
    }

    #[test]
    fn remove_peer_of_missing_entry_is_a_noop() {
        let tod = tempfile::tempdir().unwrap();
        add_peer(tod.path(), "10.0.0.5").unwrap();

        remove_peer(tod.path(), "10.0.0.9").unwrap();

        assert_eq!(list_peers(tod.path()).unwrap(), vec!["10.0.0.5".to_string()]);
    }

    #[test]
    fn get_listen_port_reads_real_shape() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_main_ini(tod.path());

        assert_eq!(get_listen_port(tod.path()).unwrap(), 47584);
    }

    #[test]
    fn get_listen_port_defaults_when_key_absent() {
        let tod = tempfile::tempdir().unwrap();
        let settings = tod.path().join("steam_settings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(settings.join("configs.main.ini"), "[main::connectivity]\r\noffline=0\r\n").unwrap();

        assert_eq!(get_listen_port(tod.path()).unwrap(), DEFAULT_LISTEN_PORT);
    }

    #[test]
    fn get_listen_port_defaults_when_file_absent() {
        let tod = tempfile::tempdir().unwrap();
        assert_eq!(get_listen_port(tod.path()).unwrap(), DEFAULT_LISTEN_PORT);
    }

    #[test]
    fn set_listen_port_writes_custom_value() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_main_ini(tod.path());

        set_listen_port(tod.path(), 30000).unwrap();

        assert_eq!(get_listen_port(tod.path()).unwrap(), 30000);
    }

    #[test]
    fn apply_preset_default_resets_to_vendor_default() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_main_ini(tod.path());
        set_listen_port(tod.path(), 30000).unwrap();

        apply_preset(tod.path(), NetworkPreset::Default).unwrap();

        assert_eq!(get_listen_port(tod.path()).unwrap(), DEFAULT_LISTEN_PORT);
    }

    #[test]
    fn apply_preset_custom_port_sets_given_value() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_main_ini(tod.path());

        apply_preset(tod.path(), NetworkPreset::CustomPort(51820)).unwrap();

        assert_eq!(get_listen_port(tod.path()).unwrap(), 51820);
    }
}
