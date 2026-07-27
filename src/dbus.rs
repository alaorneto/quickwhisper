//! D-Bus service consumed by the GNOME Shell extension (overlay + auto-paste).
//!
//! The daemon emits state signals; the extension renders the pill overlay and
//! injects Ctrl+V on `Finished`. The extension exports its own bus name so the
//! daemon can detect it and decide between overlay feedback and notifications.

use std::future::Future;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tracing::{debug, info, warn};
use zbus::blocking::connection;
use zbus::object_server::SignalEmitter;

use crate::config::OverlayMonitor;

pub const BUS_NAME: &str = "io.github.alaor.QuickWhisper";
pub const OBJECT_PATH: &str = "/io/github/alaor/QuickWhisper";
/// Exported by the extension; its presence means overlay + auto-paste work.
pub const OVERLAY_BUS_NAME: &str = "io.github.alaor.QuickWhisper.Overlay";
/// UUID of the companion GNOME Shell extension.
pub const EXTENSION_UUID: &str = "quickwhisper@alaorneto.github.io";

struct QuickWhisperIface {
    state: Arc<Mutex<String>>,
    cancel_tx: Sender<()>,
}

#[zbus::interface(name = "io.github.alaor.QuickWhisper")]
impl QuickWhisperIface {
    /// "idle" | "recording" | "processing"
    fn get_state(&self) -> String {
        self.state.lock().expect("state mutex").clone()
    }

    /// Aborts an in-progress recording, discarding the audio.
    fn cancel_recording(&self) {
        let _ = self.cancel_tx.send(());
    }

    #[zbus(signal)]
    async fn overlay_monitor_mode(
        emitter: &SignalEmitter<'_>,
        mode: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn recording_started(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    /// Normalized microphone RMS (0.0–1.0), ~15 Hz while recording.
    #[zbus(signal)]
    async fn audio_level(emitter: &SignalEmitter<'_>, level: f64) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn processing_started(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn finished(emitter: &SignalEmitter<'_>, text: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn failed(emitter: &SignalEmitter<'_>, message: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn cancelled(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

struct Service {
    conn: zbus::blocking::Connection,
    emitter: SignalEmitter<'static>,
    state: Arc<Mutex<String>>,
    overlay_monitor: OverlayMonitor,
}

/// Daemon-side handle. `None` inside means the session bus is unavailable —
/// every method degrades to a no-op so the daemon keeps working headless.
#[derive(Clone)]
pub struct DbusHandle(Option<Arc<Service>>);

impl DbusHandle {
    /// `cancel_tx` receives a unit for each CancelRecording method call.
    pub fn start(cancel_tx: Sender<()>, overlay_monitor: OverlayMonitor) -> Self {
        match Self::try_start(cancel_tx, overlay_monitor) {
            Ok(service) => DbusHandle(Some(Arc::new(service))),
            Err(e) => {
                warn!("D-Bus indisponível ({e:#}); overlay/auto-paste desativados");
                DbusHandle(None)
            }
        }
    }

    fn try_start(cancel_tx: Sender<()>, overlay_monitor: OverlayMonitor) -> Result<Service> {
        let state = Arc::new(Mutex::new("idle".to_owned()));
        let iface = QuickWhisperIface { state: Arc::clone(&state), cancel_tx };
        let conn = connection::Builder::session()?
            .name(BUS_NAME)?
            .serve_at(OBJECT_PATH, iface)?
            .build()
            .context("registrando serviço D-Bus (outro daemon já está rodando?)")?;
        let iface_ref = conn
            .object_server()
            .interface::<_, QuickWhisperIface>(OBJECT_PATH)?;
        let emitter = iface_ref.signal_emitter().clone();
        Ok(Service { conn, emitter, state, overlay_monitor })
    }

    fn set_state(&self, value: &str) {
        if let Some(s) = &self.0 {
            *s.state.lock().expect("state mutex") = value.to_owned();
        }
    }

    fn emit(&self, what: &str, fut: impl Future<Output = zbus::Result<()>>) {
        if let Err(e) = zbus::block_on(fut) {
            debug!("falha ao emitir sinal {what}: {e}");
        }
    }

    pub fn recording_started(&self) {
        self.set_state("recording");
        if let Some(s) = &self.0 {
            self.emit(
                "OverlayMonitorMode",
                QuickWhisperIface::overlay_monitor_mode(
                    &s.emitter,
                    s.overlay_monitor.as_str(),
                ),
            );
            self.emit("RecordingStarted", QuickWhisperIface::recording_started(&s.emitter));
        }
    }

    pub fn audio_level(&self, level: f64) {
        if let Some(s) = &self.0 {
            self.emit("AudioLevel", QuickWhisperIface::audio_level(&s.emitter, level));
        }
    }

    pub fn processing_started(&self) {
        self.set_state("processing");
        if let Some(s) = &self.0 {
            self.emit("ProcessingStarted", QuickWhisperIface::processing_started(&s.emitter));
        }
    }

    pub fn finished(&self, text: &str) {
        self.set_state("idle");
        if let Some(s) = &self.0 {
            self.emit("Finished", QuickWhisperIface::finished(&s.emitter, text));
        }
    }

    pub fn failed(&self, message: &str) {
        self.set_state("idle");
        if let Some(s) = &self.0 {
            self.emit("Failed", QuickWhisperIface::failed(&s.emitter, message));
        }
    }

    pub fn cancelled(&self) {
        self.set_state("idle");
        if let Some(s) = &self.0 {
            self.emit("Cancelled", QuickWhisperIface::cancelled(&s.emitter));
        }
    }

    /// Asks GNOME Shell to enable the companion extension. Idempotent; covers
    /// fresh installs where the extension files exist but nobody enabled it.
    pub fn enable_extension(&self) {
        let Some(s) = &self.0 else { return };
        let call = || -> Result<bool> {
            let proxy = zbus::blocking::Proxy::new(
                &s.conn,
                "org.gnome.Shell.Extensions",
                "/org/gnome/Shell/Extensions",
                "org.gnome.Shell.Extensions",
            )?;
            Ok(proxy.call("EnableExtension", &(EXTENSION_UUID,))?)
        };
        match call() {
            Ok(true) => info!("extensão {EXTENSION_UUID} habilitada no GNOME Shell"),
            Ok(false) => debug!("GNOME Shell recusou habilitar {EXTENSION_UUID} (não instalada?)"),
            Err(e) => debug!("EnableExtension indisponível (sessão sem GNOME Shell?): {e:#}"),
        }
    }

    /// Whether the GNOME Shell extension is on the bus right now. Checked per
    /// dictation, so enabling the extension takes effect without a restart.
    pub fn overlay_present(&self) -> bool {
        let Some(s) = &self.0 else { return false };
        let check = || -> Result<bool> {
            let proxy = zbus::blocking::fdo::DBusProxy::new(&s.conn)?;
            Ok(proxy.name_has_owner(OVERLAY_BUS_NAME.try_into()?)?)
        };
        check().unwrap_or_else(|e| {
            debug!("consulta de presença da extensão falhou: {e:#}");
            false
        })
    }
}
