use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tracing::{debug, warn};

/// Whisper expects 16 kHz mono f32 in [-1, 1].
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

pub struct Recording {
    /// 16 kHz mono samples, ready for whisper.
    pub samples: Vec<f32>,
}

/// Called with the normalized (0.0–1.0) microphone level, ~15 Hz.
pub type LevelCallback = Box<dyn Fn(f32) + Send>;

/// A microphone capture in progress on a dedicated thread.
///
/// cpal streams are !Send, so the stream lives and dies inside the thread;
/// control happens through the stop flag.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Result<Recording>>,
}

impl Recorder {
    /// Starts capturing. `on_auto_stop` fires (once) if `max` elapses before
    /// `stop()` is called — lets the daemon notice stuck-key timeouts.
    pub fn start(
        device_name: &str,
        max: Duration,
        on_auto_stop: Option<Box<dyn FnOnce() + Send>>,
        on_level: Option<LevelCallback>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let device_name = device_name.to_owned();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

        let handle = std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                capture_thread(&device_name, max, thread_stop, on_auto_stop, on_level, ready_tx)
            })
            .context("criando thread de captura")?;

        // Surface device/stream errors to the caller instead of only at stop():
        // a missing mic should fail the recording immediately.
        ready_rx
            .recv()
            .map_err(|_| anyhow!("thread de captura morreu antes de inicializar"))??;
        Ok(Self { stop, handle })
    }

    pub fn stop(self) -> Result<Recording> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .join()
            .map_err(|_| anyhow!("thread de captura entrou em pânico"))?
    }
}

fn capture_thread(
    device_name: &str,
    max: Duration,
    stop: Arc<AtomicBool>,
    on_auto_stop: Option<Box<dyn FnOnce() + Send>>,
    on_level: Option<LevelCallback>,
    ready_tx: std::sync::mpsc::Sender<Result<()>>,
) -> Result<Recording> {
    type CaptureParts = (cpal::Stream, u32, usize, Arc<Mutex<Vec<f32>>>);
    let init = || -> Result<CaptureParts> {
        let host = cpal::default_host();
        let device = if device_name == "default" {
            host.default_input_device()
        } else {
            host.input_devices()?
                .find(|d| d.description().map(|desc| desc.name() == device_name).unwrap_or(false))
        }
        .with_context(|| format!("dispositivo de entrada '{device_name}' não encontrado"))?;

        let supported = device
            .default_input_config()
            .context("consultando formato de captura")?;
        let sample_rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        debug!(
            "capturando de '{}' a {} Hz, {} canais, {:?}",
            device.description().map(|d| d.name().to_owned()).unwrap_or_default(),
            sample_rate,
            channels,
            supported.sample_format()
        );

        let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
        let cb_buffer = Arc::clone(&buffer);
        let err_cb = |e| warn!("erro no stream de áudio: {e}");
        let config = cpal::StreamConfig {
            channels: supported.channels(),
            sample_rate: supported.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                config,
                move |data: &[f32], _: &_| {
                    cb_buffer.lock().expect("audio mutex").extend_from_slice(data);
                },
                err_cb,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                config,
                move |data: &[i16], _: &_| {
                    let mut buf = cb_buffer.lock().expect("audio mutex");
                    buf.extend(data.iter().map(|&s| s as f32 / 32_768.0));
                },
                err_cb,
                None,
            )?,
            cpal::SampleFormat::U16 => device.build_input_stream(
                config,
                move |data: &[u16], _: &_| {
                    let mut buf = cb_buffer.lock().expect("audio mutex");
                    buf.extend(data.iter().map(|&s| (s as f32 - 32_768.0) / 32_768.0));
                },
                err_cb,
                None,
            )?,
            other => anyhow::bail!("formato de amostra não suportado: {other:?}"),
        };
        stream.play().context("iniciando stream de captura")?;
        Ok((stream, sample_rate, channels, buffer))
    };

    let (stream, sample_rate, channels, buffer) = match init() {
        Ok(parts) => {
            let _ = ready_tx.send(Ok(()));
            parts
        }
        Err(e) => {
            let _ = ready_tx.send(Err(anyhow!("{e:#}")));
            return Err(e);
        }
    };

    let started = Instant::now();
    let mut timed_out = false;
    let mut last_len = 0usize;
    let mut ticks = 0u32;
    while !stop.load(Ordering::Relaxed) {
        if started.elapsed() >= max {
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
        ticks += 1;
        // Feed the overlay waveform with the RMS of freshly captured samples
        // (~every 60 ms); done here, in the control loop, to keep the
        // realtime cpal callback free of extra work.
        if let Some(cb) = &on_level {
            if ticks.is_multiple_of(2) {
                let buf = buffer.lock().expect("audio mutex");
                let new = &buf[last_len.min(buf.len())..];
                if !new.is_empty() {
                    let rms =
                        (new.iter().map(|s| s * s).sum::<f32>() / new.len() as f32).sqrt();
                    last_len = buf.len();
                    drop(buf);
                    // Speech RMS is typically 0.03–0.3; scale into 0..1.
                    cb((rms * 5.0).min(1.0));
                }
            }
        }
    }
    drop(stream);
    if timed_out {
        if let Some(cb) = on_auto_stop {
            cb();
        }
    }

    let raw = std::mem::take(&mut *buffer.lock().expect("audio mutex"));
    let mono = mix_to_mono(raw, channels);
    let samples = resample(mono, sample_rate, WHISPER_SAMPLE_RATE)?;
    Ok(Recording { samples })
}

/// Takes ownership so the (common) mono case is a pass-through, not a copy of
/// the whole recording.
fn mix_to_mono(interleaved: Vec<f32>, channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved;
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn resample(input: Vec<f32>, from: u32, to: u32) -> Result<Vec<f32>> {
    use rubato::{FastFixedIn, PolynomialDegree, Resampler};

    if from == to || input.is_empty() {
        return Ok(input);
    }
    let ratio = to as f64 / from as f64;
    const CHUNK: usize = 1024;
    let mut resampler = FastFixedIn::<f32>::new(ratio, 1.0, PolynomialDegree::Cubic, CHUNK, 1)
        .context("criando resampler")?;

    let mut out = Vec::with_capacity((input.len() as f64 * ratio) as usize + CHUNK);
    let mut padded;
    for block in input.chunks(CHUNK) {
        let block: &[f32] = if block.len() == CHUNK {
            block
        } else {
            // Last partial block: pad with silence (a few ms at the tail is harmless).
            padded = block.to_vec();
            padded.resize(CHUNK, 0.0);
            &padded
        };
        let result = resampler
            .process(&[block], None)
            .context("resampleando áudio")?;
        out.extend_from_slice(&result[0]);
    }
    Ok(out)
}

/// Loads a WAV file and converts it to whisper input format (16 kHz mono f32).
pub fn load_wav_as_whisper_input(path: &Path) -> Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("abrindo {}", path.display()))?;
    let spec = reader.spec();
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| s as f32 / max))
                .collect::<Result<_, _>>()?
        }
    };
    let mono = mix_to_mono(raw, spec.channels as usize);
    resample(mono, spec.sample_rate, WHISPER_SAMPLE_RATE)
}

/// Writes 16 kHz mono f32 samples as a 16-bit PCM WAV (playable anywhere).
pub fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: WHISPER_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).with_context(|| format!("criando {}", path.display()))?;
    for &s in samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * 32_767.0) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}
