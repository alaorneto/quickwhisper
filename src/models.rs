use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use crate::config;

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

pub fn model_path(name: &str) -> Result<PathBuf> {
    Ok(config::models_dir()?.join(format!("ggml-{name}.bin")))
}

/// Returns the model path, failing with an actionable message if it was never downloaded.
pub fn require(name: &str) -> Result<PathBuf> {
    let path = model_path(name)?;
    if !path.exists() {
        bail!(
            "modelo '{name}' não encontrado em {}.\nBaixe com: quickwhisper model download {name}",
            path.display()
        );
    }
    Ok(path)
}

pub fn list() -> Result<()> {
    let dir = config::models_dir()?;
    let mut found = false;
    if dir.exists() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("ggml-") && name.ends_with(".bin") {
                let mb = entry.metadata().map(|m| m.len() / 1_048_576).unwrap_or(0);
                println!("{} ({mb} MB)", &name[5..name.len() - 4]);
                found = true;
            }
        }
    }
    if !found {
        println!("Nenhum modelo baixado. Ex.: quickwhisper model download small");
    }
    Ok(())
}

pub fn download(name: &str) -> Result<PathBuf> {
    let path = model_path(name)?;
    if path.exists() {
        println!("Modelo já existe em {}", path.display());
        return Ok(path);
    }
    let dir = path.parent().expect("model path sempre tem diretório");
    std::fs::create_dir_all(dir)?;

    let url = format!("{BASE_URL}/ggml-{name}.bin");
    println!("Baixando {url}…");

    let mut response = ureq::get(&url)
        .call()
        .with_context(|| format!("baixando {url} — o nome do modelo está correto?"))?;
    let total: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    // Download to a temp name and rename at the end so an interrupted download
    // never masquerades as a valid model file.
    let tmp = path.with_extension("bin.part");
    let mut file = std::fs::File::create(&tmp)?;
    let mut reader = response.body_mut().as_reader();
    let mut buf = [0u8; 1024 * 128];
    let mut done: u64 = 0;
    let mut last_pct: u64 = u64::MAX;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if let Some(total) = total {
            let pct = done * 100 / total;
            if pct != last_pct {
                eprint!("\r{pct}% ({} / {} MB)", done / 1_048_576, total / 1_048_576);
                last_pct = pct;
            }
        }
    }
    eprintln!();
    file.flush()?;
    drop(file);
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}
