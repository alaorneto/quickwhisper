use std::path::Path;

use anyhow::{Context, Result};
use tracing::info;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::WHISPER_SAMPLE_RATE;

pub struct Transcriber {
    ctx: WhisperContext,
}

pub struct Transcription {
    pub text: String,
    /// Detected language code (when language was "auto"), if whisper reports one.
    pub language: Option<String>,
}

impl Transcriber {
    pub fn load(model_path: &Path) -> Result<Self> {
        // Route whisper.cpp/ggml stderr chatter through `tracing` so the
        // default env filter keeps the journal clean.
        whisper_rs::install_logging_hooks();
        let path = model_path
            .to_str()
            .context("caminho do modelo não é UTF-8 válido")?;
        info!("carregando modelo {path}");
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .with_context(|| format!("carregando modelo {path}"))?;
        Ok(Self { ctx })
    }

    pub fn transcribe(&self, samples: &[f32], language: &str) -> Result<Transcription> {
        // whisper.cpp misbehaves on sub-second inputs; pad to slightly over 1s.
        let min_len = (WHISPER_SAMPLE_RATE as usize * 11) / 10;
        let padded;
        let samples = if samples.len() < min_len {
            padded = {
                let mut p = samples.to_vec();
                p.resize(min_len, 0.0);
                p
            };
            &padded
        } else {
            samples
        };

        let mut state = self.ctx.create_state().context("criando estado do whisper")?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4)
            .min(8); // whisper.cpp scales poorly past 8 threads
        params.set_n_threads(threads);
        params.set_language(Some(language)); // "auto" enables detection
        params.set_translate(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_no_context(true);

        state.full(params, samples).context("executando whisper")?;

        let n = state.full_n_segments();
        let mut text = String::new();
        for i in 0..n {
            if let Some(segment) = state.get_segment(i) {
                text.push_str(&segment.to_str_lossy().context("decodificando segmento")?);
            }
        }
        let text = text.trim().to_owned();

        let language = if language == "auto" {
            whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_owned)
        } else {
            None
        };

        Ok(Transcription { text, language })
    }
}
