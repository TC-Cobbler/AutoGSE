//! Phase 7 §7.8.8 built the "Launch lobby_connect" entry point over
//! `engine::run_join`/`goldberg::run_lobby_connect` (§6.7) — the dashboard's
//! per-card Join Lobby icon (`autogse_gui.rs::trigger_join_lobby`) calls the
//! same thing, this dialog is a second entry point for a game not currently
//! in the scanned grid.
//!
//! Phase 9 §9.1/§9.2 adds everything below: this target's real
//! `custom_broadcasts.txt` peer list and `listen_port` (`autogse::lan`,
//! mutated directly rather than through `engine::run_lan`'s CLI/println
//! wrapper, same "GUI calls the lower-level module directly" convention
//! `save_sync_view.rs` already established), plus VPN adapter/Tailscale
//! peer detection (`autogse::vpn_adapters`). Every operation here can touch
//! disk (peer-list/INI edits) or the network/an external process (Tailscale
//! CLI), so all of it runs on a background thread, converging back via
//! `slint::invoke_from_event_loop` like every other Phase 7.8/8/9 dialog.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;

use autogse::cli::JoinArgs;
use autogse::error::AutoGseError;
use autogse::{discovery, engine, lan, manifest, vpn_adapters};

use crate::gui_interaction::GuiInteraction;
use crate::{MultiplayerDialog, TailscalePeerRow};

pub type DialogHolder = Rc<RefCell<Option<MultiplayerDialog>>>;

pub fn new_holder() -> DialogHolder {
    Rc::new(RefCell::new(None))
}

pub fn open(holder: &DialogHolder) -> Result<(), String> {
    let dialog = MultiplayerDialog::new().map_err(|e| e.to_string())?;

    {
        let weak = dialog.as_weak();
        dialog.on_launch_requested(move |path_text| {
            let Some(dialog) = weak.upgrade() else { return };
            let path = PathBuf::from(path_text.as_str());
            dialog.set_launching(true);
            dialog.set_status_message(format!("Launching lobby_connect for {}...", path.display()).into());

            let weak_for_thread = weak.clone();
            std::thread::spawn(move || {
                let interaction = GuiInteraction;
                let result = engine::run_join(&JoinArgs { path: path.clone() }, &interaction);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(dialog) = weak_for_thread.upgrade() else { return };
                    let message = match result {
                        Ok(()) => format!("lobby_connect closed for {}.", path.display()),
                        Err(e) => format!("Error: {e}"),
                    };
                    dialog.set_status_message(message.into());
                    dialog.set_launching(false);
                });
            });
        });
    }

    {
        let weak = dialog.as_weak();
        dialog.on_refresh_requested(move |path_text| run_refresh(weak.clone(), path_text.to_string()));
    }
    {
        let weak = dialog.as_weak();
        dialog.on_add_peer_requested(move |ip| run_peer_action(weak.clone(), PeerAction::Add(ip.to_string())));
    }
    {
        let weak = dialog.as_weak();
        dialog.on_remove_peer_requested(move |ip| run_peer_action(weak.clone(), PeerAction::Remove(ip.to_string())));
    }
    {
        let weak = dialog.as_weak();
        dialog.on_set_listen_port_requested(move |port_text| run_set_listen_port(weak.clone(), port_text.to_string()));
    }
    {
        let weak = dialog.as_weak();
        dialog.on_detect_vpn_requested(move || run_detect_vpn(weak.clone()));
    }

    {
        let holder = holder.clone();
        dialog.window().on_close_requested(move || {
            holder.borrow_mut().take();
            slint::CloseRequestResponse::HideWindow
        });
    }

    let _ = dialog.show();
    *holder.borrow_mut() = Some(dialog);
    Ok(())
}

enum PeerAction {
    Add(String),
    Remove(String),
}

/// Resolves `path_text` to an already-injected target's TOD, erroring the
/// same way `engine::run_lan` does (`AutoGseError::NotInjected`) if it isn't
/// one yet — every LAN action shares this same resolution step.
fn resolve_injected_tod(path_text: &str) -> Result<PathBuf, AutoGseError> {
    let resolution = discovery::resolve_target(&PathBuf::from(path_text), None)?;
    if manifest::load(&resolution.tod)?.is_none() {
        return Err(AutoGseError::NotInjected(resolution.tod));
    }
    Ok(resolution.tod)
}

fn run_refresh(weak: slint::Weak<MultiplayerDialog>, path_text: String) {
    let Some(dialog) = weak.upgrade() else { return };
    dialog.set_status_message("Loading this target's network settings...".into());

    let weak_for_thread = weak.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<(Vec<String>, u16), AutoGseError> {
            let tod = resolve_injected_tod(&path_text)?;
            Ok((lan::list_peers(&tod)?, lan::get_listen_port(&tod)?))
        })();

        let _ = slint::invoke_from_event_loop(move || {
            let Some(dialog) = weak_for_thread.upgrade() else { return };
            match result {
                Ok((peers, port)) => {
                    let peers: Vec<slint::SharedString> = peers.into_iter().map(Into::into).collect();
                    dialog.set_broadcast_peers(slint::ModelRc::new(slint::VecModel::from(peers)));
                    dialog.set_listen_port_text(port.to_string().into());
                    dialog.set_status_message("Loaded.".into());
                }
                Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
            }
        });
    });
}

fn run_peer_action(weak: slint::Weak<MultiplayerDialog>, action: PeerAction) {
    let Some(dialog) = weak.upgrade() else { return };
    let path_text = dialog.get_target_path().to_string();
    dialog.set_status_message("Working...".into());

    let weak_for_thread = weak.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<String>, AutoGseError> {
            let tod = resolve_injected_tod(&path_text)?;
            match &action {
                PeerAction::Add(ip) => lan::add_peer(&tod, ip)?,
                PeerAction::Remove(ip) => lan::remove_peer(&tod, ip)?,
            }
            lan::list_peers(&tod)
        })();

        let _ = slint::invoke_from_event_loop(move || {
            let Some(dialog) = weak_for_thread.upgrade() else { return };
            match result {
                Ok(peers) => {
                    let peers: Vec<slint::SharedString> = peers.into_iter().map(Into::into).collect();
                    dialog.set_broadcast_peers(slint::ModelRc::new(slint::VecModel::from(peers)));
                    dialog.set_new_peer_text("".into());
                    dialog.set_status_message("Updated broadcast peer list.".into());
                }
                Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
            }
        });
    });
}

fn run_set_listen_port(weak: slint::Weak<MultiplayerDialog>, port_text: String) {
    let Some(dialog) = weak.upgrade() else { return };
    let target_text = dialog.get_target_path().to_string();
    dialog.set_status_message("Working...".into());

    let weak_for_thread = weak.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<u16, AutoGseError> {
            let port: u16 = port_text
                .trim()
                .parse()
                .map_err(|_| AutoGseError::Lan(format!("'{port_text}' is not a valid port number (0-65535)")))?;
            let tod = resolve_injected_tod(&target_text)?;
            lan::set_listen_port(&tod, port)?;
            lan::get_listen_port(&tod)
        })();

        let _ = slint::invoke_from_event_loop(move || {
            let Some(dialog) = weak_for_thread.upgrade() else { return };
            match result {
                Ok(port) => {
                    dialog.set_listen_port_text(port.to_string().into());
                    dialog.set_status_message("Listen port updated.".into());
                }
                Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
            }
        });
    });
}

fn run_detect_vpn(weak: slint::Weak<MultiplayerDialog>) {
    if let Some(dialog) = weak.upgrade() {
        dialog.set_detecting_vpn(true);
        dialog.set_status_message("Detecting VPN adapters...".into());
    }

    let weak_for_thread = weak.clone();
    std::thread::spawn(move || {
        let adapters = vpn_adapters::detect_vpn_adapters().unwrap_or_default();
        let peers = vpn_adapters::tailscale_peers().unwrap_or_default();

        let adapter_labels: Vec<slint::SharedString> = adapters
            .iter()
            .map(|a| slint::SharedString::from(format!("{} — {}", a.kind.label(), a.ipv4.map(|ip| ip.to_string()).unwrap_or_else(|| "(no IPv4 address)".to_string()))))
            .collect();
        let peer_rows: Vec<TailscalePeerRow> = peers
            .iter()
            .map(|p| TailscalePeerRow {
                label: format!("{} ({}){}", p.hostname, p.ip, if p.online { "" } else { " — offline" }).into(),
                ip: p.ip.clone().into(),
            })
            .collect();

        let _ = slint::invoke_from_event_loop(move || {
            let Some(dialog) = weak_for_thread.upgrade() else { return };
            dialog.set_vpn_adapters(slint::ModelRc::new(slint::VecModel::from(adapter_labels)));
            dialog.set_tailscale_peers(slint::ModelRc::new(slint::VecModel::from(peer_rows)));
            dialog.set_detecting_vpn(false);
            dialog.set_status_message("VPN detection complete.".into());
        });
    });
}
