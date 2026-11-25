//! Download tests against a local HTTP server. No real network.

use puw_app::DataDirs;
use puw_app::download::{
    Asset, DownloadError, DownloadProgress, download, download_with_backoff, download_with_options,
    missing, model_assets_at, remove_downloaded_data,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const NONE: Duration = Duration::ZERO;

fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// An asset whose expected hash is that of `bytes`.
fn asset(server: &Server, name: &'static str, bytes: &[u8]) -> Asset {
    Asset {
        name,
        url: format!("{}/{name}", server.base),
        sha256: Box::leak(sha_hex(bytes).into_boxed_str()),
        bytes: bytes.len() as u64,
    }
}

#[derive(Debug, Clone)]
struct Req {
    path: String,
    range: Option<String>,
}

#[derive(Default)]
struct Behavior {
    /// Answer 200 with the whole file even when a range is asked for.
    ignore_range: bool,
    /// Serve the file with one byte flipped.
    corrupt: bool,
    /// Serve 404 for everything.
    not_found: bool,
}

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<Req>>>,
    behavior: Arc<Mutex<Behavior>>,
    /// How many of the next responses are cut off after half of the body.
    cut_next: Arc<AtomicUsize>,
    /// Like `cut_next`, but the connection goes silent instead of closing.
    stall_next: Arc<AtomicUsize>,
    release: Arc<AtomicBool>,
    server: Arc<tiny_http::Server>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// A body that fails after `limit` bytes, which drops the connection.
struct Cut {
    inner: io::Cursor<Vec<u8>>,
    limit: usize,
    sent: usize,
    /// Go silent at the limit instead of failing, until released.
    stall: Option<Arc<AtomicBool>>,
}

impl Read for Cut {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.sent >= self.limit {
            if let Some(release) = &self.stall {
                // Silent but connected; bounded so a broken test cannot hang.
                for _ in 0..500 {
                    if release.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "cut"));
        }
        let max = buf.len().min(self.limit - self.sent);
        let n = self.inner.read(&mut buf[..max])?;
        self.sent += n;
        Ok(n)
    }
}

impl Server {
    fn start(files: Vec<(&'static str, Vec<u8>)>) -> Server {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let files: HashMap<String, Vec<u8>> = files
            .into_iter()
            .map(|(n, d)| (format!("/{n}"), d))
            .collect();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let behavior = Arc::new(Mutex::new(Behavior::default()));
        let cut_next = Arc::new(AtomicUsize::new(0));
        let stall_next = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let thread = {
            let (server, requests, behavior, cut_next, stall_next, release) = (
                server.clone(),
                requests.clone(),
                behavior.clone(),
                cut_next.clone(),
                stall_next.clone(),
                release.clone(),
            );
            std::thread::spawn(move || {
                for request in server.incoming_requests() {
                    let range = request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Range"))
                        .map(|h| h.value.to_string());
                    let path = request.url().to_string();
                    requests.lock().unwrap().push(Req {
                        path: path.clone(),
                        range: range.clone(),
                    });
                    let (ignore_range, corrupt, not_found) = {
                        let b = behavior.lock().unwrap();
                        (b.ignore_range, b.corrupt, b.not_found)
                    };
                    let Some(body) = files.get(&path).filter(|_| !not_found) else {
                        let _ = request.respond(tiny_http::Response::empty(404));
                        continue;
                    };
                    let mut body = body.clone();
                    if corrupt {
                        let mid = body.len() / 2;
                        body[mid] ^= 0xff;
                    }
                    let (status, start) = match range.as_deref() {
                        Some(r) if !ignore_range => {
                            let from: usize = r
                                .trim_start_matches("bytes=")
                                .trim_end_matches('-')
                                .parse()
                                .unwrap();
                            (206, from)
                        }
                        _ => (200, 0),
                    };
                    let total = body.len();
                    let part = body[start..].to_vec();
                    let len = part.len();
                    let mut headers = Vec::new();
                    if status == 206 {
                        headers.push(
                            tiny_http::Header::from_bytes(
                                "Content-Range",
                                format!("bytes {start}-{}/{total}", total - 1),
                            )
                            .unwrap(),
                        );
                    }
                    let cut = take_one(&cut_next);
                    let stall = take_one(&stall_next);
                    let limit = if cut || stall { len / 2 } else { len };
                    let reader = Cut {
                        inner: io::Cursor::new(part),
                        limit,
                        sent: 0,
                        stall: stall.then(|| release.clone()),
                    };
                    let response = tiny_http::Response::new(
                        tiny_http::StatusCode(status),
                        headers,
                        reader,
                        Some(len),
                        None,
                    );
                    // Own thread, so a stalled response does not block the next request.
                    std::thread::spawn(move || {
                        let _ = request.respond(response);
                    });
                }
            })
        };
        Server {
            base,
            requests,
            behavior,
            cut_next,
            stall_next,
            release,
            server,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> Vec<Req> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.release.store(true, Ordering::SeqCst);
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Decrement `n` unless it is zero; true when it was decremented.
fn take_one(n: &AtomicUsize) -> bool {
    let mut cur = n.load(Ordering::SeqCst);
    while cur > 0 {
        match n.compare_exchange(cur, cur - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return true,
            Err(now) => cur = now,
        }
    }
    false
}

fn no_progress(_: DownloadProgress) {}

fn run(dir: &Path, assets: &[Asset]) -> Result<(), DownloadError> {
    download_with_backoff(dir, assets, &no_progress, &AtomicBool::new(false), &[])
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

#[test]
fn downloads_and_verifies() {
    let a = data(300_000, 1);
    let b = data(1000, 2);
    let s = Server::start(vec![("a.bin", a.clone()), ("b.bin", b.clone())]);
    let assets = [asset(&s, "a.bin", &a), asset(&s, "b.bin", &b)];
    let t = tempfile::tempdir().unwrap();
    assert_eq!(missing(t.path(), &assets).len(), 2);
    run(t.path(), &assets).unwrap();
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
    assert_eq!(std::fs::read(t.path().join("b.bin")).unwrap(), b);
    assert_eq!(
        std::fs::read_to_string(t.path().join("a.bin.sha256"))
            .unwrap()
            .trim(),
        assets[0].sha256
    );
    assert_eq!(
        file_names(t.path()),
        ["a.bin", "a.bin.sha256", "b.bin", "b.bin.sha256"]
    );
    assert!(missing(t.path(), &assets).is_empty());
}

#[test]
fn skips_present_files() {
    let a = data(5000, 1);
    let b = data(6000, 2);
    let s = Server::start(vec![("a.bin", a.clone()), ("b.bin", b.clone())]);
    let assets = [asset(&s, "a.bin", &a), asset(&s, "b.bin", &b)];
    let t = tempfile::tempdir().unwrap();
    run(t.path(), &assets[..1]).unwrap();
    let before = s.requests().len();
    run(t.path(), &assets).unwrap();
    let after = s.requests();
    assert_eq!(after.len(), before + 1);
    assert_eq!(after.last().unwrap().path, "/b.bin");
    let again = s.requests().len();
    run(t.path(), &assets).unwrap();
    assert_eq!(s.requests().len(), again);
}

#[test]
fn present_needs_matching_size_and_hash_file() {
    let a = data(5000, 1);
    let s = Server::start(vec![("a.bin", a.clone())]);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    run(t.path(), &assets).unwrap();
    // A wrong size is missing.
    std::fs::write(t.path().join("a.bin"), &a[..100]).unwrap();
    assert_eq!(missing(t.path(), &assets).len(), 1);
    // The right size without a recorded hash is missing too.
    std::fs::write(t.path().join("a.bin"), &a).unwrap();
    assert!(missing(t.path(), &assets).is_empty());
    std::fs::remove_file(t.path().join("a.bin.sha256")).unwrap();
    assert_eq!(missing(t.path(), &assets).len(), 1);
    // A recorded hash for something else is missing.
    std::fs::write(t.path().join("a.bin.sha256"), "00").unwrap();
    assert_eq!(missing(t.path(), &assets).len(), 1);
}

#[test]
fn resumes_partial_download() {
    let a = data(400_000, 3);
    let s = Server::start(vec![("a.bin", a.clone())]);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    s.cut_next.store(1, Ordering::SeqCst);
    let err = run(t.path(), &assets).unwrap_err();
    assert!(matches!(err, DownloadError::Network(_)), "{err:?}");
    assert!(!t.path().join("a.bin").exists());
    let part = std::fs::metadata(t.path().join("a.bin.part"))
        .unwrap()
        .len();
    assert!(part > 0 && part < a.len() as u64, "part = {part}");
    run(t.path(), &assets).unwrap();
    let reqs = s.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].range, None);
    assert_eq!(reqs[1].range, Some(format!("bytes={part}-")));
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
    assert!(!t.path().join("a.bin.part").exists());
}

#[test]
fn retries_within_one_call() {
    let a = data(100_000, 4);
    let s = Server::start(vec![("a.bin", a.clone())]);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    s.cut_next.store(2, Ordering::SeqCst);
    download_with_backoff(
        t.path(),
        &assets,
        &no_progress,
        &AtomicBool::new(false),
        &[NONE, NONE],
    )
    .unwrap();
    assert_eq!(s.requests().len(), 3);
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
}

#[test]
fn gives_up_after_three_attempts() {
    let a = data(1000, 4);
    let s = Server::start(vec![("a.bin", a.clone())]);
    s.behavior.lock().unwrap().not_found = true;
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    let err = download_with_backoff(
        t.path(),
        &assets,
        &no_progress,
        &AtomicBool::new(false),
        &[NONE, NONE],
    )
    .unwrap_err();
    assert!(matches!(err, DownloadError::Network(_)), "{err:?}");
    assert_eq!(s.requests().len(), 3);
    assert!(file_names(t.path()).is_empty());
}

#[test]
fn server_ignores_range_restarts() {
    let a = data(200_000, 5);
    let s = Server::start(vec![("a.bin", a.clone())]);
    s.behavior.lock().unwrap().ignore_range = true;
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    // Leave a genuine half-file behind.
    std::fs::write(t.path().join("a.bin.part"), &a[..80_000]).unwrap();
    run(t.path(), &assets).unwrap();
    let reqs = s.requests();
    assert_eq!(reqs[0].range.as_deref(), Some("bytes=80000-"));
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
}

#[test]
fn oversized_part_is_started_over() {
    let a = data(10_000, 5);
    let s = Server::start(vec![("a.bin", a.clone())]);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("a.bin.part"), data(20_000, 9)).unwrap();
    run(t.path(), &assets).unwrap();
    assert_eq!(s.requests()[0].range, None);
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
}

#[test]
fn complete_part_is_verified_without_a_request() {
    let a = data(10_000, 6);
    let s = Server::start(vec![("a.bin", a.clone())]);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("a.bin.part"), &a).unwrap();
    run(t.path(), &assets).unwrap();
    assert!(s.requests().is_empty());
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
}

#[test]
fn corrupt_file_is_redownloaded() {
    let a = data(50_000, 7);
    let s = Server::start(vec![("a.bin", a.clone())]);
    s.behavior.lock().unwrap().corrupt = true;
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    let err = run(t.path(), &assets).unwrap_err();
    assert!(
        matches!(&err, DownloadError::Verify { name } if name == "a.bin"),
        "{err:?}"
    );
    assert!(
        file_names(t.path()).is_empty(),
        "{:?}",
        file_names(t.path())
    );
    assert_eq!(s.requests().len(), 1, "a bad hash is not retried");

    s.behavior.lock().unwrap().corrupt = false;
    run(t.path(), &assets).unwrap();
    assert_eq!(s.requests().len(), 2);
    assert_eq!(s.requests()[1].range, None);
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
}

#[test]
fn corrupt_part_from_an_earlier_run_is_caught() {
    let a = data(50_000, 7);
    let s = Server::start(vec![("a.bin", a.clone())]);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    let mut bad = a[..20_000].to_vec();
    bad[5] ^= 1;
    std::fs::write(t.path().join("a.bin.part"), bad).unwrap();
    let err = run(t.path(), &assets).unwrap_err();
    assert!(matches!(err, DownloadError::Verify { .. }), "{err:?}");
    assert!(file_names(t.path()).is_empty());
    run(t.path(), &assets).unwrap();
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
}

#[test]
fn rejects_bad_names() {
    let a = data(100, 1);
    let s = Server::start(vec![("a.bin", a.clone())]);
    for bad in ["../x", "a/b", "a\\b", "", "..", "."] {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("models");
        let mut asset = asset(&s, "a.bin", &a);
        asset.name = bad;
        // A valid asset before the bad one must not be fetched either.
        let ok = self::asset(&s, "a.bin", &a);
        let err = run(&dir, &[ok, asset]).unwrap_err();
        assert!(
            matches!(&err, DownloadError::BadName(n) if n == bad),
            "{err:?}"
        );
        assert!(!dir.exists(), "{bad:?} created {dir:?}");
        assert!(file_names(t.path()).is_empty());
    }
    assert!(s.requests().is_empty());
}

#[test]
fn cancel_keeps_part() {
    let a = data(600_000, 8);
    let s = Server::start(vec![("a.bin", a.clone())]);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let on_progress = |p: DownloadProgress| {
        if p.done_bytes >= 100_000 {
            cancel.store(true, Ordering::SeqCst);
        }
    };
    let err =
        download_with_backoff(t.path(), &assets, &on_progress, &cancel, &[NONE, NONE]).unwrap_err();
    assert!(matches!(err, DownloadError::Cancelled), "{err:?}");
    let part = std::fs::metadata(t.path().join("a.bin.part"))
        .unwrap()
        .len();
    assert!(part >= 100_000 && part < a.len() as u64, "part = {part}");
    assert!(!t.path().join("a.bin").exists());
    assert_eq!(s.requests().len(), 1, "cancel is not retried");

    // The kept part is picked up again.
    cancel.store(false, Ordering::SeqCst);
    download_with_backoff(t.path(), &assets, &no_progress, &cancel, &[]).unwrap();
    assert_eq!(s.requests()[1].range, Some(format!("bytes={part}-")));
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
}

#[test]
fn cancelled_before_start_does_nothing() {
    let a = data(1000, 8);
    let s = Server::start(vec![("a.bin", a.clone())]);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    let err = download_with_backoff(t.path(), &assets, &no_progress, &AtomicBool::new(true), &[])
        .unwrap_err();
    assert!(matches!(err, DownloadError::Cancelled));
    assert!(s.requests().is_empty());
}

#[test]
fn progress_is_monotonic() {
    let a = data(300_000, 1);
    let b = data(50_000, 2);
    let s = Server::start(vec![("a.bin", a.clone()), ("b.bin", b.clone())]);
    let assets = [asset(&s, "a.bin", &a), asset(&s, "b.bin", &b)];
    let t = tempfile::tempdir().unwrap();
    // A cut and a restart in the middle must not move progress backwards.
    s.cut_next.store(1, Ordering::SeqCst);
    s.behavior.lock().unwrap().ignore_range = true;
    let seen = Mutex::new(Vec::<DownloadProgress>::new());
    let on_progress = |p: DownloadProgress| seen.lock().unwrap().push(p);
    download_with_backoff(
        t.path(),
        &assets,
        &on_progress,
        &AtomicBool::new(false),
        &[NONE],
    )
    .unwrap();
    let seen = seen.into_inner().unwrap();
    assert!(seen.len() > 2);
    let total = (a.len() + b.len()) as u64;
    let mut last = 0;
    for p in &seen {
        assert_eq!(p.total_bytes, total);
        assert!(p.done_bytes >= last, "{} after {last}", p.done_bytes);
        assert!(p.done_bytes <= total);
        last = p.done_bytes;
    }
    assert_eq!(last, total);
    assert_eq!(seen.last().unwrap().file, "b.bin");
}

#[test]
fn progress_counts_files_already_present() {
    let a = data(5000, 1);
    let b = data(7000, 2);
    let s = Server::start(vec![("a.bin", a.clone()), ("b.bin", b.clone())]);
    let assets = [asset(&s, "a.bin", &a), asset(&s, "b.bin", &b)];
    let t = tempfile::tempdir().unwrap();
    run(t.path(), &assets[..1]).unwrap();
    let seen = Mutex::new(Vec::<DownloadProgress>::new());
    let on_progress = |p: DownloadProgress| seen.lock().unwrap().push(p);
    download(t.path(), &assets, &on_progress, &AtomicBool::new(false)).unwrap();
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen[0].total_bytes, 12_000);
    assert!(seen[0].done_bytes >= 5000);
    assert_eq!(seen.last().unwrap().done_bytes, 12_000);
}

#[test]
fn progress_serializes_camel_case() {
    let p = DownloadProgress {
        done_bytes: 1,
        total_bytes: 2,
        file: "x".into(),
    };
    assert_eq!(
        serde_json::to_string(&p).unwrap(),
        r#"{"doneBytes":1,"totalBytes":2,"file":"x"}"#
    );
}

#[test]
fn unusable_folder_is_a_disk_error() {
    let a = data(100, 1);
    let s = Server::start(vec![("a.bin", a.clone())]);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    let blocker = t.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let err = run(&blocker, &assets).unwrap_err();
    assert!(matches!(err, DownloadError::Disk(_)), "{err:?}");
}

#[test]
fn model_assets_have_the_release_values() {
    let list = model_assets_at("http://h/p/");
    let got: Vec<_> = list
        .iter()
        .map(|a| (a.name, a.url.as_str(), a.bytes, a.sha256))
        .collect();
    assert_eq!(
        got,
        [
            (
                "segmenter.onnx",
                "http://h/p/segmenter.onnx",
                480_487_402,
                "2af5fa2a3ae7afd8594489dad3e9042fe1d37cddc77a399e5a6e1f6923d5e77d"
            ),
            (
                "lama.onnx",
                "http://h/p/lama.onnx",
                209_009_184,
                "844672871fc83d16c6fa27c230424299423a8e2f2e821d9552a63fa5736939cd"
            ),
            (
                "face.onnx",
                "http://h/p/face.onnx",
                232_589,
                "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4"
            ),
        ]
    );
    let real = puw_app::download::model_assets();
    assert_eq!(
        real[0].url,
        "https://github.com/007hacky007/PhotoUnwatermark/releases/download/models-v2/segmenter.onnx"
    );
    assert_eq!(real.len(), 3);
}

#[test]
fn remove_downloaded_data_keeps_settings() {
    let t = tempfile::tempdir().unwrap();
    let dirs = DataDirs::at(&t.path().join("data"));
    dirs.ensure().unwrap();
    std::fs::create_dir_all(&dirs.cache).unwrap();
    std::fs::write(dirs.models.join("lama.onnx"), "m").unwrap();
    std::fs::write(dirs.watermarks.join("w.png"), "w").unwrap();
    std::fs::write(dirs.cache.join("c.bin"), "c").unwrap();
    std::fs::write(&dirs.settings, "{}").unwrap();
    remove_downloaded_data(&dirs).unwrap();
    assert!(!dirs.models.exists() && !dirs.watermarks.exists() && !dirs.cache.exists());
    assert_eq!(std::fs::read_to_string(&dirs.settings).unwrap(), "{}");
    // Nothing there is fine too.
    remove_downloaded_data(&dirs).unwrap();
}

const SHORT: Duration = Duration::from_millis(300);

#[test]
fn stalled_connection_fails_and_keeps_part() {
    let a = data(200_000, 3);
    let s = Server::start(vec![("a.bin", a.clone())]);
    s.stall_next.store(1, Ordering::SeqCst);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let err = download_with_options(
        t.path(),
        &assets,
        &no_progress,
        &AtomicBool::new(false),
        &[],
        SHORT,
    )
    .unwrap_err();
    assert!(matches!(err, DownloadError::Network(_)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let part = std::fs::metadata(t.path().join("a.bin.part"))
        .unwrap()
        .len();
    assert!(part > 0 && part < a.len() as u64, "part = {part}");
}

#[test]
fn stalled_connection_is_retried_and_resumed() {
    let a = data(200_000, 3);
    let s = Server::start(vec![("a.bin", a.clone())]);
    s.stall_next.store(1, Ordering::SeqCst);
    let assets = [asset(&s, "a.bin", &a)];
    let t = tempfile::tempdir().unwrap();
    download_with_options(
        t.path(),
        &assets,
        &no_progress,
        &AtomicBool::new(false),
        &[NONE],
        SHORT,
    )
    .unwrap();
    let reqs = s.requests();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[1].range.is_some());
    assert_eq!(std::fs::read(t.path().join("a.bin")).unwrap(), a);
}
