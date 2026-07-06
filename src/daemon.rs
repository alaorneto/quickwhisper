//! The push-to-talk daemon: a small synchronous state machine.
//!
//! Phase 1 deliberately avoids async — hotkeys arrive on an mpsc channel from
//! evdev threads, audio capture runs on its own thread, and whisper blocks the
//! loop on purpose (presses during processing are stale and get drained).

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::{error, info, warn};

use crate::config::{Config, HotkeyMode};
use crate::{audio, clipboard, hotkey, models, transcribe};

/// Recordings shorter than this are treated as accidental taps and discarded.
const MIN_RECORDING: Duration = Duration::from_millis(300);

#[derive(Debug)]
enum Event {
    Key(hotkey::HotkeyEvent),
    /// The recorder hit the max-duration safety cap on its own.
    RecorderTimeout,
}

pub fn run(cfg: Config) -> Result<()> {
    let model_path = models::require(&cfg.whisper.model)?;
    // Loading takes seconds; do it once at startup and keep it resident so a
    // dictation only pays inference time.
    let transcriber = transcribe::Transcriber::load(&model_path)?;

    let key = hotkey::parse_key(&cfg.hotkey.key)?;
    let (tx, rx) = mpsc::channel::<Event>();
    let key_tx = tx.clone();
    let (raw_tx, raw_rx) = mpsc::channel::<hotkey::HotkeyEvent>();
    let devices = hotkey::spawn_listeners(key, raw_tx)?;
    std::thread::spawn(move || {
        for ev in raw_rx {
            if key_tx.send(Event::Key(ev)).is_err() {
                return;
            }
        }
    });

    info!(
        "quickwhisper pronto: segure {} para ditar (modo {:?}, modelo {}, {} teclado(s))",
        cfg.hotkey.key, cfg.hotkey.mode, cfg.whisper.model, devices
    );
    notify(
        "QuickWhisper ativo",
        &format!("Segure {} para ditar. O texto vai para o clipboard.", cfg.hotkey.key),
    );

    let mut recorder: Option<(audio::Recorder, Instant)> = None;
    for event in &rx {
        let start_wanted = matches!(
            (&cfg.hotkey.mode, &event, recorder.is_some()),
            (HotkeyMode::PushToTalk, Event::Key(hotkey::HotkeyEvent::Pressed), false)
                | (HotkeyMode::Toggle, Event::Key(hotkey::HotkeyEvent::Pressed), false)
        );
        let stop_wanted = matches!(
            (&cfg.hotkey.mode, &event, recorder.is_some()),
            (HotkeyMode::PushToTalk, Event::Key(hotkey::HotkeyEvent::Released), true)
                | (HotkeyMode::Toggle, Event::Key(hotkey::HotkeyEvent::Pressed), true)
                | (_, Event::RecorderTimeout, true)
        );

        if start_wanted {
            let timeout_tx: Sender<Event> = tx.clone();
            let max = Duration::from_secs(cfg.audio.max_recording_secs);
            match audio::Recorder::start(
                &cfg.audio.device,
                max,
                Some(Box::new(move || {
                    let _ = timeout_tx.send(Event::RecorderTimeout);
                })),
            ) {
                Ok(r) => {
                    info!("gravando…");
                    recorder = Some((r, Instant::now()));
                }
                Err(e) => {
                    error!("falha ao iniciar captura: {e:#}");
                    notify("QuickWhisper — erro no microfone", &format!("{e:#}"));
                }
            }
        } else if stop_wanted {
            let (rec, started) = recorder.take().expect("stop_wanted implica recorder ativo");
            let held = started.elapsed();
            match rec.stop() {
                Ok(recording) if held < MIN_RECORDING => {
                    info!("toque acidental ({held:?}); descartado");
                    drop(recording);
                }
                Ok(recording) => {
                    process_recording(&transcriber, &cfg, recording);
                    drain_stale_keys(&rx);
                }
                Err(e) => {
                    error!("falha na captura: {e:#}");
                    notify("QuickWhisper — erro na gravação", &format!("{e:#}"));
                }
            }
        }
    }
    Ok(())
}

fn process_recording(transcriber: &transcribe::Transcriber, cfg: &Config, rec: audio::Recording) {
    let secs = rec.samples.len() as f64 / audio::WHISPER_SAMPLE_RATE as f64;
    info!("transcrevendo {secs:.1}s de áudio…");
    let t0 = Instant::now();
    match transcriber.transcribe(&rec.samples, &cfg.whisper.language) {
        Ok(result) if result.text.is_empty() => {
            info!("transcrição vazia (silêncio?); nada copiado");
        }
        Ok(result) => {
            info!(
                "transcrito em {:.1}s{}: {}",
                t0.elapsed().as_secs_f64(),
                result.language.as_deref().map(|l| format!(" [{l}]")).unwrap_or_default(),
                result.text
            );
            match clipboard::set_text(&result.text) {
                Ok(()) => notify("Transcrito e copiado 📋", &result.text),
                Err(e) => {
                    error!("clipboard falhou: {e:#}");
                    notify("QuickWhisper — falha no clipboard", &result.text);
                }
            }
        }
        Err(e) => {
            error!("whisper falhou: {e:#}");
            notify("QuickWhisper — erro na transcrição", &format!("{e:#}"));
        }
    }
}

/// Discards key events that queued up while whisper was running so a press
/// made during processing doesn't spuriously start a new recording.
fn drain_stale_keys(rx: &Receiver<Event>) {
    let mut n = 0;
    while rx.try_recv().is_ok() {
        n += 1;
    }
    if n > 0 {
        warn!("{n} evento(s) de tecla ignorado(s) durante o processamento");
    }
}

/// Fire-and-forget GNOME notification; failure must never break dictation.
fn notify(summary: &str, body: &str) {
    let summary = summary.to_owned();
    let body = body.to_owned();
    std::thread::spawn(move || {
        let result = notify_rust::Notification::new()
            .appname("QuickWhisper")
            .summary(&summary)
            .body(&body)
            .icon("audio-input-microphone")
            .timeout(notify_rust::Timeout::Milliseconds(5000))
            .show();
        if let Err(e) = result {
            warn!("notificação falhou: {e}");
        }
    });
}
