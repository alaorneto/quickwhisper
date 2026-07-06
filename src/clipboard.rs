use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use tracing::{debug, warn};

/// Whether the data-control protocol already failed once — e.g. GNOME/Mutter
/// on Rocky 10 does not expose it to regular clients. Sticky per process so we
/// don't retry (and re-warn) on every dictation.
static LIB_UNSUPPORTED: AtomicBool = AtomicBool::new(false);

/// Sets the Wayland clipboard.
///
/// Primary path: wl-clipboard-rs, serving the selection from a detached thread
/// (the thread exits when another client takes the selection over). If the
/// compositor rejects the data-control protocol, falls back to the `wl-copy`
/// binary, which daemonizes itself.
pub fn set_text(text: &str) -> Result<()> {
    if LIB_UNSUPPORTED.load(Ordering::Relaxed) {
        return copy_via_wl_copy(text);
    }
    match copy_via_lib(text) {
        Ok(()) => Ok(()),
        Err(e) => {
            warn!("wl-clipboard-rs indisponível ({e:#}); usando wl-copy daqui em diante");
            LIB_UNSUPPORTED.store(true, Ordering::Relaxed);
            copy_via_wl_copy(text)
        }
    }
}

fn copy_via_lib(text: &str) -> Result<()> {
    use wl_clipboard_rs::copy::{MimeType, Options, Source};

    let mut options = Options::new();
    options.foreground(true);
    let prepared = options
        .prepare_copy(Source::Bytes(text.as_bytes().into()), MimeType::Text)
        .context("preparando cópia via protocolo data-control")?;
    // prepare_copy already owns the selection; serving requests must not block
    // the daemon's state machine, so it happens on its own thread.
    std::thread::Builder::new()
        .name("clipboard-serve".into())
        .spawn(move || {
            if let Err(e) = prepared.serve() {
                debug!("clipboard serve terminou: {e:#}");
            }
        })
        .context("criando thread do clipboard")?;
    Ok(())
}

fn copy_via_wl_copy(text: &str) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .context("executando wl-copy — o pacote wl-clipboard está instalado?")?;
    child
        .stdin
        .take()
        .context("stdin do wl-copy indisponível")?
        .write_all(text.as_bytes())?;
    let status = child.wait()?;
    anyhow::ensure!(status.success(), "wl-copy saiu com {status}");
    Ok(())
}
