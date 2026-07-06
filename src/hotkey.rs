//! Global push-to-talk key detection via evdev.
//!
//! Why evdev and not the GlobalShortcuts portal: Rocky Linux 10 ships
//! xdg-desktop-portal-gnome 47 built without the GlobalShortcuts backend
//! (verified 2026-07: the interface is absent from the session bus), so the
//! portal path is not available. Reading /dev/input directly gives exact
//! press/release pairs and works on any session type; it requires the user
//! to be in the `input` group.

use std::sync::mpsc::Sender;

use anyhow::{bail, Context, Result};
use evdev::{Device, EventType, KeyCode};
use tracing::{debug, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

/// Maps a friendly name ("F12") to an evdev key code ("KEY_F12").
pub fn parse_key(name: &str) -> Result<KeyCode> {
    let upper = name.to_uppercase();
    let full = if upper.starts_with("KEY_") { upper.clone() } else { format!("KEY_{upper}") };
    full.parse::<KeyCode>()
        .map_err(|_| anyhow::anyhow!("tecla desconhecida: '{name}' (tente por ex. F12, F9, PAUSE)"))
}

/// Counts keyboards we can open that have the key — used by `status`.
pub fn accessible_devices(key: KeyCode) -> Result<usize> {
    Ok(keyboards_with_key(key)?.len())
}

/// Spawns one reader thread per keyboard that has the key; press/release
/// events for `key` are forwarded to `tx`. Returns the device count.
pub fn spawn_listeners(key: KeyCode, tx: Sender<HotkeyEvent>) -> Result<usize> {
    let devices = keyboards_with_key(key)?;
    if devices.is_empty() {
        bail!(
            "nenhum teclado acessível com a tecla {key:?}.\n\
             Sem acesso a /dev/input? Adicione-se ao grupo 'input' e relogue:\n\
             sudo usermod -aG input $USER"
        );
    }
    let count = devices.len();
    for (path, device) in devices {
        let tx = tx.clone();
        let name = device.name().unwrap_or("?").to_owned();
        debug!("escutando {key:?} em {} ({name})", path.display());
        std::thread::Builder::new()
            .name(format!("evdev-{}", path.display()))
            .spawn(move || read_loop(device, key, tx))
            .context("criando thread evdev")?;
    }
    Ok(count)
}

fn keyboards_with_key(key: KeyCode) -> Result<Vec<(std::path::PathBuf, Device)>> {
    let mut found = Vec::new();
    for (path, device) in evdev::enumerate() {
        let has_key = device
            .supported_keys()
            .map(|keys| keys.contains(key))
            .unwrap_or(false);
        if has_key {
            found.push((path, device));
        }
    }
    Ok(found)
}

fn read_loop(mut device: Device, key: KeyCode, tx: Sender<HotkeyEvent>) {
    loop {
        let events = match device.fetch_events() {
            Ok(events) => events,
            Err(e) => {
                // Device unplugged or permissions revoked; this listener retires.
                warn!("leitura evdev encerrada: {e}");
                return;
            }
        };
        for event in events {
            if event.event_type() != EventType::KEY || event.code() != key.0 {
                continue;
            }
            // value: 1 = press, 0 = release, 2 = kernel auto-repeat (ignored —
            // push-to-talk only cares about edges).
            let msg = match event.value() {
                1 => HotkeyEvent::Pressed,
                0 => HotkeyEvent::Released,
                _ => continue,
            };
            if tx.send(msg).is_err() {
                return; // daemon gone
            }
        }
    }
}
