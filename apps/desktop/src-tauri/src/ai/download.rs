//! Fetching the local AI: the llama.cpp server (Vulkan build, MIT) and the
//! model (Qwen3 4B Instruct, Apache 2.0), both pinned to an exact release
//! and checked against their SHA-256. Resumes a partial download, so
//! closing Mimo or losing the connection only costs what's left.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CHUNK: usize = 256 * 1024;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

pub struct Asset {
    url: &'static str,
    /// Where it's saved, in the AI folder.
    file: &'static str,
    size: u64,
    sha256: &'static str,
}

pub const ENGINE: Asset = Asset {
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b11430/llama-b11430-bin-win-vulkan-x64.zip",
    file: "llama.zip",
    size: 33_337_688,
    sha256: "fea0653c6eab7e3e1abfa4785d033db831818b483d6912e372d2c1a99babed0a",
};

pub const MODEL: Asset = Asset {
    url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/a06e946bb6b655725eafa393f4a9745d460374c9/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
    file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
    size: 2_497_281_120,
    sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
};

/// The engine, unpacked.
pub const ENGINE_DIR: &str = "engine";
pub const SERVER_EXE: &str = "llama-server.exe";
/// Written once everything is in place; its content names this exact
/// engine + model, so changing either here fetches them again.
const MARKER: &str = "installed.txt";
const MARKER_CONTENT: &str = "llama.cpp b11430 vulkan + Qwen3-4B-Instruct-2507-Q4_K_M";

pub fn model_path(dir: &Path) -> std::path::PathBuf {
    dir.join(MODEL.file)
}

pub fn installed(dir: &Path) -> bool {
    fs::read_to_string(dir.join(MARKER)).is_ok_and(|m| m == MARKER_CONTENT)
        && dir.join(ENGINE_DIR).join(SERVER_EXE).exists()
        && fs::metadata(model_path(dir)).is_ok_and(|m| m.len() == MODEL.size)
}

pub fn total_size() -> u64 {
    ENGINE.size + MODEL.size
}

pub enum Failure {
    Cancelled,
    Failed(String),
}

impl From<String> for Failure {
    fn from(err: String) -> Self {
        Failure::Failed(err)
    }
}

/// Downloads and unpacks whatever is missing. `progress(done, total)` is
/// called a few times a second.
pub fn install(dir: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<(), Failure> {
    fs::create_dir_all(dir).map_err(|err| format!("can't create {}: {err}", dir.display()))?;
    let _ = fs::remove_file(dir.join(MARKER));
    let total = total_size();

    let engine_ready = dir.join(ENGINE_DIR).join(SERVER_EXE).exists();
    if !engine_ready {
        fetch(&ENGINE, dir, cancel, &mut |done| progress(done, total))?;
        unpack(&dir.join(ENGINE.file), &dir.join(ENGINE_DIR))?;
        let _ = fs::remove_file(dir.join(ENGINE.file));
    }
    let model_ready = fs::metadata(model_path(dir)).is_ok_and(|m| m.len() == MODEL.size);
    if !model_ready {
        fetch(&MODEL, dir, cancel, &mut |done| progress(ENGINE.size + done, total))?;
    }
    progress(total, total);
    fs::write(dir.join(MARKER), MARKER_CONTENT).map_err(|err| format!("can't write the marker: {err}"))?;
    Ok(())
}

/// Downloads `asset` into `dir` (via a `.part` file, resumed if present),
/// hashing as it goes; renamed into place only once the hash matches.
fn fetch(asset: &Asset, dir: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<(), Failure> {
    let dest = dir.join(asset.file);
    let part = dir.join(format!("{}.part", asset.file));
    let mut hasher = Sha256::new();

    // What's already there counts (and gets hashed) first.
    let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > asset.size {
        have = 0;
    }
    if have > 0 {
        let mut file = File::open(&part).map_err(|err| format!("can't read {}: {err}", part.display()))?;
        let mut buffer = vec![0; CHUNK];
        let mut left = have;
        while left > 0 {
            if cancel.load(Ordering::SeqCst) {
                return Err(Failure::Cancelled);
            }
            let n = file.read(&mut buffer[..CHUNK.min(left as usize)]).map_err(|err| err.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            left -= n as u64;
        }
        progress(have);
    }

    if have < asset.size {
        let mut request = ureq::get(asset.url);
        if have > 0 {
            request = request.header("Range", &format!("bytes={have}-"));
        }
        let mut response = request
            .config()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .build()
            .call()
            .map_err(|err| format!("download of {} failed: {err}", asset.file))?;
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&part)
            .map_err(|err| format!("can't write {}: {err}", part.display()))?;
        if have > 0 && response.status() != 206 {
            // The server sent the whole file: start over.
            have = 0;
            hasher = Sha256::new();
        }
        file.set_len(have).map_err(|err| err.to_string())?;
        let mut file = std::io::BufWriter::with_capacity(CHUNK, file);
        {
            use std::io::Seek;
            file.seek(std::io::SeekFrom::Start(have)).map_err(|err| err.to_string())?;
        }

        let mut reader = response.body_mut().as_reader();
        let mut buffer = vec![0; CHUNK];
        let mut last_report = Instant::now();
        loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = file.flush();
                return Err(Failure::Cancelled);
            }
            let n = reader.read(&mut buffer).map_err(|err| format!("download of {} interrupted: {err}", asset.file))?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(|err| format!("can't write {}: {err}", part.display()))?;
            hasher.update(&buffer[..n]);
            have += n as u64;
            if last_report.elapsed() >= PROGRESS_EVERY {
                progress(have);
                last_report = Instant::now();
            }
        }
        file.flush().map_err(|err| err.to_string())?;
        progress(have);
    }

    if have != asset.size {
        return Err(Failure::Failed(format!("{}: got {have} bytes of {}", asset.file, asset.size)));
    }
    let hash = format!("{:x}", hasher.finalize());
    if hash != asset.sha256 {
        let _ = fs::remove_file(&part);
        return Err(Failure::Failed(format!("{}: checksum mismatch", asset.file)));
    }
    fs::rename(&part, &dest).map_err(|err| format!("can't finish {}: {err}", dest.display()))?;
    Ok(())
}

/// Unpacks a zip with Windows' own `Expand-Archive` (paths go through
/// environment variables, never the command line).
fn unpack(zip: &Path, to: &Path) -> Result<(), String> {
    let temp = to.with_extension("tmp");
    let _ = fs::remove_dir_all(&temp);
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ErrorActionPreference = 'Stop'; Expand-Archive -LiteralPath $env:MIMO_ZIP -DestinationPath $env:MIMO_TO -Force",
        ])
        .env("MIMO_ZIP", zip)
        .env("MIMO_TO", &temp)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("can't run PowerShell: {err}"))?;
    if !output.status.success() {
        return Err(format!("can't unpack the engine: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    let _ = fs::remove_dir_all(to);
    fs::rename(&temp, to).map_err(|err| format!("can't move the engine into place: {err}"))
}
