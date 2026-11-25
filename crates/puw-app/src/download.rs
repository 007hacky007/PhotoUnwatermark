//! The one-time model download: resumable, verified, cancellable.

use crate::{DataDirs, Result};
use puw_core::Error;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Where the models are published.
const RELEASE_URL: &str =
    "https://github.com/007hacky007/PhotoUnwatermark/releases/download/models-v2/";

/// Waits before the second and third attempt at a file.
const BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];

/// How long a download may go without receiving any data before it counts as stalled.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

const CHUNK: usize = 64 * 1024;

/// A file to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// File name inside the target folder.
    pub name: &'static str,
    pub url: String,
    /// Lower-case hex SHA-256 of the whole file.
    pub sha256: &'static str,
    pub bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("network error: {0}")]
    Network(String),
    #[error("{name} does not match its checksum")]
    Verify { name: String },
    #[error("disk error: {0}")]
    Disk(String),
    #[error("download cancelled")]
    Cancelled,
    #[error("unusable file name: {0:?}")]
    BadName(String),
}

/// How far the download has got, across all files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub file: String,
}

/// The three model files from the project's release.
pub fn model_assets() -> Vec<Asset> {
    model_assets_at(RELEASE_URL)
}

/// The same files served from another base URL (used by tests).
pub fn model_assets_at(base_url: &str) -> Vec<Asset> {
    let base = base_url.trim_end_matches('/');
    let item = |name: &'static str, sha256: &'static str, bytes: u64| Asset {
        name,
        url: format!("{base}/{name}"),
        sha256,
        bytes,
    };
    vec![
        item(
            "segmenter.onnx",
            "2af5fa2a3ae7afd8594489dad3e9042fe1d37cddc77a399e5a6e1f6923d5e77d",
            480_487_402,
        ),
        item(
            "lama.onnx",
            "844672871fc83d16c6fa27c230424299423a8e2f2e821d9552a63fa5736939cd",
            209_009_184,
        ),
        item(
            "face.onnx",
            "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
            232_589,
        ),
    ]
}

fn hash_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.sha256"))
}

fn part_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.part"))
}

fn is_present(dir: &Path, asset: &Asset) -> bool {
    let size_ok = std::fs::metadata(dir.join(asset.name))
        .map(|m| m.is_file() && m.len() == asset.bytes)
        .unwrap_or(false);
    size_ok
        && std::fs::read_to_string(hash_path(dir, asset.name))
            .map(|h| h.trim() == asset.sha256)
            .unwrap_or(false)
}

/// The assets that still need downloading. A file counts as present when it has the
/// right size and its `.sha256` note (written only after a verified download) holds
/// the expected hash.
pub fn missing(dir: &Path, assets: &[Asset]) -> Vec<Asset> {
    assets
        .iter()
        .filter(|a| !is_present(dir, a))
        .cloned()
        .collect()
}

fn check_name(name: &str) -> std::result::Result<(), DownloadError> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        || name.contains(':');
    if bad {
        Err(DownloadError::BadName(name.to_string()))
    } else {
        Ok(())
    }
}

/// Download what is missing into `dir`, with up to three attempts per file.
///
/// Partial files are kept as `<name>.part` and continued next time. A file is only
/// renamed to its final name after its SHA-256 matches.
pub fn download(
    dir: &Path,
    assets: &[Asset],
    on_progress: &(dyn Fn(DownloadProgress) + Sync),
    cancel: &AtomicBool,
) -> std::result::Result<(), DownloadError> {
    download_with_backoff(dir, assets, on_progress, cancel, &BACKOFF)
}

/// [`download`] with explicit waits between attempts; there is one more attempt than
/// there are waits.
pub fn download_with_backoff(
    dir: &Path,
    assets: &[Asset],
    on_progress: &(dyn Fn(DownloadProgress) + Sync),
    cancel: &AtomicBool,
    backoff: &[Duration],
) -> std::result::Result<(), DownloadError> {
    download_with_options(dir, assets, on_progress, cancel, backoff, READ_TIMEOUT)
}

/// [`download_with_backoff`] with an explicit stall limit: a connection that delivers
/// nothing for `read_timeout` fails as a network error (and is retried and resumed).
pub fn download_with_options(
    dir: &Path,
    assets: &[Asset],
    on_progress: &(dyn Fn(DownloadProgress) + Sync),
    cancel: &AtomicBool,
    backoff: &[Duration],
    read_timeout: Duration,
) -> std::result::Result<(), DownloadError> {
    for a in assets {
        check_name(a.name)?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(DownloadError::Cancelled);
    }
    std::fs::create_dir_all(dir).map_err(disk)?;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .user_agent(concat!("PhotoUnwatermark/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_per_read(Some(read_timeout))
        .build()
        .into();

    let total_bytes = assets.iter().map(|a| a.bytes).sum();
    let mut progress = Progress {
        on_progress,
        total_bytes,
        high: 0,
    };
    let mut base = 0;
    for asset in assets {
        if is_present(dir, asset) {
            base += asset.bytes;
            progress.emit(base, asset.name);
            continue;
        }
        let mut attempt = 0;
        loop {
            match fetch(&agent, dir, asset, base, &mut progress, cancel) {
                Err(DownloadError::Network(_)) if attempt < backoff.len() => {
                    sleep_unless_cancelled(backoff[attempt], cancel)?;
                    attempt += 1;
                }
                other => {
                    other?;
                    break;
                }
            }
        }
        base += asset.bytes;
    }
    Ok(())
}

fn disk(e: std::io::Error) -> DownloadError {
    DownloadError::Disk(e.to_string())
}

fn sleep_unless_cancelled(
    wait: Duration,
    cancel: &AtomicBool,
) -> std::result::Result<(), DownloadError> {
    let step = Duration::from_millis(50);
    let mut left = wait;
    while !left.is_zero() {
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }
        let now = left.min(step);
        std::thread::sleep(now);
        left -= now;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(DownloadError::Cancelled);
    }
    Ok(())
}

/// Reports progress that never goes backwards, even when a file restarts.
struct Progress<'a> {
    on_progress: &'a (dyn Fn(DownloadProgress) + Sync),
    total_bytes: u64,
    high: u64,
}

impl Progress<'_> {
    fn emit(&mut self, done: u64, file: &str) {
        self.high = self.high.max(done).min(self.total_bytes);
        (self.on_progress)(DownloadProgress {
            done_bytes: self.high,
            total_bytes: self.total_bytes,
            file: file.to_string(),
        });
    }
}

/// One attempt at one file: continue or start the `.part`, verify, move into place.
fn fetch(
    agent: &ureq::Agent,
    dir: &Path,
    asset: &Asset,
    base: u64,
    progress: &mut Progress,
    cancel: &AtomicBool,
) -> std::result::Result<(), DownloadError> {
    let final_path = dir.join(asset.name);
    let part = part_path(dir, asset.name);
    // Whatever was there is no longer vouched for.
    remove_if_exists(&hash_path(dir, asset.name))?;

    let mut hasher = Sha256::new();
    let mut have = match std::fs::metadata(&part) {
        Ok(m) if m.is_file() && m.len() <= asset.bytes => m.len(),
        Ok(_) => {
            remove_if_exists(&part)?;
            0
        }
        Err(_) => 0,
    };
    // Shown before the part is read again, which takes a few seconds for hundreds of MB.
    progress.emit(base + have, asset.name);
    if have > 0 {
        let mut f = File::open(&part).map_err(disk)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf).map_err(disk)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            if cancel.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
        }
    }

    if have < asset.bytes {
        have = transfer(
            agent,
            asset,
            &part,
            have,
            &mut hasher,
            base,
            progress,
            cancel,
        )?;
    }
    if have != asset.bytes {
        // More than expected: it cannot be the right file.
        remove_if_exists(&part)?;
        return Err(DownloadError::Verify {
            name: asset.name.to_string(),
        });
    }

    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if digest != asset.sha256 {
        remove_if_exists(&part)?;
        return Err(DownloadError::Verify {
            name: asset.name.to_string(),
        });
    }
    std::fs::rename(&part, &final_path).map_err(disk)?;
    std::fs::write(hash_path(dir, asset.name), format!("{}\n", asset.sha256)).map_err(disk)?;
    progress.emit(base + asset.bytes, asset.name);
    Ok(())
}

fn remove_if_exists(path: &Path) -> std::result::Result<(), DownloadError> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(disk(e)),
        _ => Ok(()),
    }
}

/// Request the rest of the file from byte `have` and append it to `part`, returning
/// the new length. A server that answers 200 gets the file restarted from zero.
#[allow(clippy::too_many_arguments)]
fn transfer(
    agent: &ureq::Agent,
    asset: &Asset,
    part: &Path,
    have: u64,
    hasher: &mut Sha256,
    base: u64,
    progress: &mut Progress,
    cancel: &AtomicBool,
) -> std::result::Result<u64, DownloadError> {
    let mut request = agent.get(&asset.url);
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
    }
    let response = request
        .call()
        .map_err(|e| DownloadError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    let mut file;
    let mut written;
    match status {
        206 => {
            let starts_right = response
                .headers()
                .get("Content-Range")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with(&format!("bytes {have}-")));
            if !starts_right {
                remove_if_exists(part)?;
                return Err(DownloadError::Network(
                    "the server sent the wrong part of the file".into(),
                ));
            }
            file = OpenOptions::new().append(true).open(part).map_err(disk)?;
            file.seek(SeekFrom::End(0)).map_err(disk)?;
            written = have;
        }
        200 => {
            file = File::create(part).map_err(disk)?;
            *hasher = Sha256::new();
            written = 0;
        }
        416 => {
            // The partial file does not fit what the server has.
            remove_if_exists(part)?;
            return Err(DownloadError::Network(
                "the partial file was rejected".into(),
            ));
        }
        other => return Err(DownloadError::Network(format!("server answered {other}"))),
    }

    let mut body = response.into_body().into_reader();
    let mut buf = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }
        let n = body
            .read(&mut buf)
            .map_err(|e| DownloadError::Network(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(disk)?;
        hasher.update(&buf[..n]);
        written += n as u64;
        progress.emit(base + written, asset.name);
        if written > asset.bytes {
            break;
        }
    }
    file.flush().map_err(disk)?;
    if written < asset.bytes {
        return Err(DownloadError::Network("the download ended early".into()));
    }
    Ok(written)
}

/// Delete the downloaded models, the watermark pictures and the model cache. The
/// settings file stays.
pub fn remove_downloaded_data(dirs: &DataDirs) -> Result<()> {
    for dir in [&dirs.models, &dirs.watermarks, &dirs.cache] {
        match std::fs::remove_dir_all(dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(Error::Io(e).into()),
            _ => {}
        }
    }
    Ok(())
}
