mod audio;
mod clipboard;
mod config;
mod daemon;
mod history;
mod hotkey;
mod models;
mod transcribe;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "quickwhisper", version, about = "Push-to-talk voice transcription for GNOME/Wayland")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the push-to-talk daemon (hold the hotkey, speak, release)
    Daemon,
    /// Manage whisper models
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// Transcribe a WAV file and report timing (debug/benchmark)
    Transcribe {
        /// Path to a WAV file (any sample rate; mono or stereo)
        file: PathBuf,
        /// Model to use (overrides config), e.g. "base" or "small"
        #[arg(long)]
        model: Option<String>,
    },
    /// Record from the microphone to a WAV file (mic test)
    Record {
        /// Seconds to record
        #[arg(long, default_value_t = 4)]
        secs: u64,
        /// Output WAV path (16 kHz mono, same format fed to whisper)
        #[arg(long, default_value = "recording-test.wav")]
        out: PathBuf,
    },
    /// Print press/release events for the configured hotkey (evdev test)
    HotkeyTest,
    /// Show configuration and environment diagnostics
    Status,
    /// Read or change configuration (e.g. `config set language auto`)
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Browse past transcriptions (list, show, copy, delete)
    History {
        #[command(subcommand)]
        command: HistoryCommand,
    },
}

#[derive(Subcommand)]
enum HistoryCommand {
    /// List transcriptions, newest first
    List {
        /// Maximum entries to show
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Full entries as JSON (timestamps in UTC ISO 8601)
        #[arg(long)]
        json: bool,
    },
    /// Print the full text of one transcription
    Show { id: i64 },
    /// Copy a transcription back to the clipboard
    Copy { id: i64 },
    /// Delete one or more transcriptions
    Delete {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// Delete the whole history
    Clear {
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the whole configuration
    Show,
    /// Print one value (keys: key, mode, device, max_recording_secs, model, language)
    Get { key: String },
    /// Change one value, e.g.: `config set language pt` or `config set language auto`
    Set { key: String, value: String },
}

#[derive(Subcommand)]
enum ModelCommand {
    /// Download a ggml model to the local models directory
    Download {
        /// One of: tiny, base, small, medium, large-v3, or a -q5_1 quantized variant
        name: String,
    },
    /// List downloaded models
    List,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "quickwhisper=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let cfg = config::Config::load()?;

    match cli.command {
        Command::Daemon => daemon::run(cfg),
        Command::Model { command } => match command {
            ModelCommand::Download { name } => {
                let path = models::download(&name)?;
                println!("Modelo salvo em {}", path.display());
                Ok(())
            }
            ModelCommand::List => models::list(),
        },
        Command::Transcribe { file, model } => {
            let model = model.unwrap_or(cfg.whisper.model);
            cmd_transcribe(&file, &model, &cfg.whisper.language)
        }
        Command::Record { secs, out } => cmd_record(&cfg, secs, &out),
        Command::HotkeyTest => cmd_hotkey_test(&cfg),
        Command::Status => cmd_status(&cfg),
        Command::Config { command } => cmd_config(cfg, command),
        Command::History { command } => cmd_history(command),
    }
}

fn cmd_history(command: HistoryCommand) -> Result<()> {
    let history = history::History::open()?;
    match command {
        HistoryCommand::List { limit, json } => {
            let entries = history.list(limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if entries.is_empty() {
                println!("Histórico vazio — dite algo com o daemon rodando.");
            } else {
                for e in &entries {
                    println!(
                        "{:>4}  {}  {:>4}  {}",
                        e.id,
                        e.created_at_local,
                        e.lang.as_deref().unwrap_or("?"),
                        preview(&e.text, 60)
                    );
                }
            }
        }
        HistoryCommand::Show { id } => println!("{}", history.get(id)?.text),
        HistoryCommand::Copy { id } => {
            let entry = history.get(id)?;
            clipboard::set_text_oneshot(&entry.text)?;
            println!("Copiado para o clipboard: {}", preview(&entry.text, 60));
        }
        HistoryCommand::Delete { ids } => {
            let deleted = history.delete(&ids)?;
            println!("{deleted} transcrição(ões) apagada(s)");
        }
        HistoryCommand::Clear { yes } => {
            let count = history.count()?;
            if count == 0 {
                println!("Histórico já está vazio.");
                return Ok(());
            }
            if !yes {
                eprint!("Apagar todas as {count} transcrições? [s/N] ");
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !matches!(answer.trim().to_lowercase().as_str(), "s" | "sim" | "y" | "yes") {
                    println!("Cancelado.");
                    return Ok(());
                }
            }
            println!("{} transcrição(ões) apagada(s)", history.clear()?);
        }
    }
    Ok(())
}

/// First line of `text`, truncated to `max` characters (UTF-8 safe).
fn preview(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max || text.lines().count() > 1 {
        out.push('…');
    }
    out
}

fn cmd_config(mut cfg: config::Config, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Show => print!("{}", toml::to_string_pretty(&cfg)?),
        ConfigCommand::Get { key } => println!("{}", cfg.get_by_key(&key)?),
        ConfigCommand::Set { key, value } => {
            cfg.set_by_key(&key, &value)?;
            cfg.save()?;
            println!("{key} = {value}");
            println!("(reinicie o daemon para aplicar)");
        }
    }
    Ok(())
}

fn cmd_transcribe(file: &std::path::Path, model: &str, language: &str) -> Result<()> {
    use std::time::Instant;

    let samples = audio::load_wav_as_whisper_input(file)?;
    let audio_secs = samples.len() as f64 / 16_000.0;
    println!("Áudio: {:.1}s ({} amostras a 16 kHz mono)", audio_secs, samples.len());

    let model_path = models::require(model)?;
    let t0 = Instant::now();
    let transcriber = transcribe::Transcriber::load(&model_path)?;
    let load_time = t0.elapsed();

    let t1 = Instant::now();
    let result = transcriber.transcribe(&samples, language)?;
    let run_time = t1.elapsed();

    println!("\n--- Transcrição ({model}) ---\n{}\n", result.text);
    if let Some(lang) = result.language {
        println!("Idioma detectado: {lang}");
    }
    println!(
        "Carga do modelo: {:.2}s · Transcrição: {:.2}s ({:.2}x tempo real)",
        load_time.as_secs_f64(),
        run_time.as_secs_f64(),
        run_time.as_secs_f64() / audio_secs
    );
    Ok(())
}

fn cmd_record(cfg: &config::Config, secs: u64, out: &std::path::Path) -> Result<()> {
    use std::time::Duration;

    println!("Gravando {secs}s do dispositivo '{}'…", cfg.audio.device);
    let recorder = audio::Recorder::start(&cfg.audio.device, Duration::from_secs(secs), None)?;
    std::thread::sleep(Duration::from_secs(secs));
    let recording = recorder.stop()?;

    let peak = recording.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    audio::write_wav(out, &recording.samples)?;
    println!(
        "Gravado {:.1}s · pico de amplitude {:.3} {} · salvo em {}",
        recording.samples.len() as f64 / 16_000.0,
        peak,
        if peak < 0.01 { "(⚠ silêncio? verifique o microfone)" } else { "(ok)" },
        out.display()
    );
    Ok(())
}

fn cmd_hotkey_test(cfg: &config::Config) -> Result<()> {
    use std::sync::mpsc;

    let key = hotkey::parse_key(&cfg.hotkey.key)?;
    let (tx, rx) = mpsc::channel();
    let devices = hotkey::spawn_listeners(key, tx)?;
    println!(
        "Escutando {key:?} em {devices} dispositivo(s). Pressione e solte {} (Ctrl+C para sair)…",
        cfg.hotkey.key
    );
    loop {
        match rx.recv()? {
            hotkey::HotkeyEvent::Pressed => println!("⬇ press"),
            hotkey::HotkeyEvent::Released => println!("⬆ release"),
        }
    }
}

fn cmd_status(cfg: &config::Config) -> Result<()> {
    println!("config : {}", config::config_path()?.display());
    println!("hotkey : {} (modo {:?})", cfg.hotkey.key, cfg.hotkey.mode);
    println!("modelo : {}", cfg.whisper.model);
    match models::model_path(&cfg.whisper.model) {
        Ok(p) if p.exists() => {
            let mb = std::fs::metadata(&p).map(|m| m.len() / 1_048_576).unwrap_or(0);
            println!("         {} ({mb} MB) ✓", p.display());
        }
        Ok(p) => println!(
            "         {} AUSENTE — rode: quickwhisper model download {}",
            p.display(),
            cfg.whisper.model
        ),
        Err(e) => println!("         erro: {e}"),
    }
    println!(
        "wayland: {}",
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "AUSENTE (sessão não-Wayland?)".into())
    );
    match hotkey::parse_key(&cfg.hotkey.key).and_then(hotkey::accessible_devices) {
        Ok(n) if n > 0 => println!("evdev  : {n} teclado(s) com acesso ✓"),
        Ok(_) => println!("evdev  : nenhum dispositivo acessível — você está no grupo 'input'?"),
        Err(e) => println!("evdev  : {e}"),
    }
    Ok(())
}
