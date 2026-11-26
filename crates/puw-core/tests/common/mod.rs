//! Helpers shared by the integration tests.
#![allow(dead_code)]

use puw_core::engine::EngineOptions;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Engine options as the app ships them, for the tests that check output quality: Core ML on
/// Macs (it falls back to the processor where Core ML is not used, such as Intel Macs), the
/// processor elsewhere (the Windows CI runner has no GPU). Tests of processor behaviour build
/// their own options.
pub fn shipping_options() -> EngineOptions {
    EngineOptions {
        prefer_gpu: cfg!(target_os = "macos"),
        cache_dir: Some(test_cache_dir()),
        ..EngineOptions::default()
    }
}

/// Compiled-model cache shared by the model tests of this crate, in `<workspace>/target`. Without
/// a cache folder, Core ML compiles into a new temp folder on every load and never removes it.
pub fn test_cache_dir() -> PathBuf {
    workspace_root()
        .join("target")
        .join("test-model-cache")
        .join("puw-core")
}

/// Folder with the model files, from `PUW_MODELS_DIR`. Tests that need models return early
/// with `eprintln!("skipped: PUW_MODELS_DIR not set")` when this is `None`.
pub fn models_dir() -> Option<PathBuf> {
    std::env::var_os("PUW_MODELS_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root above crates/puw-core")
}

/// Synthetic test sets in `<workspace>/target/synth`, generated with tools/synth/make_sets.py
/// when the folder is missing or incomplete.
///
/// A finished folder holds a `.complete` marker, written after generation succeeds. Without it
/// the folder is treated as incomplete and rebuilt: a CI cache restore of `target/` can bring
/// back an empty folder tree whose files were not cached, and a bare existence check would
/// accept it.
///
/// Tests in one binary share a single generation through a `OnceLock`. Separate test binaries
/// may race, so each generates into its own `target/synth.tmp-<pid>` and renames it into place;
/// the loser of a race deletes its copy and uses the folder that won.
pub fn synth_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = workspace_root();
        let dir = root.join("target").join("synth");
        if !dir.join(COMPLETE_MARKER).is_file() {
            // Only a folder without the marker is removed, so a complete one is never deleted.
            if dir.exists() && !dir.join(COMPLETE_MARKER).is_file() {
                let _ = std::fs::remove_dir_all(&dir);
            }
            generate_into(root, &dir);
        }
        dir
    })
    .clone()
}

const COMPLETE_MARKER: &str = ".complete";

fn generate_into(root: &Path, dir: &Path) {
    let tmp = dir.with_file_name(format!("synth.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let status = Command::new("uv")
        .args([
            "run",
            "--project",
            "tools",
            "python",
            "tools/synth/make_sets.py",
            "--out",
        ])
        .arg(&tmp)
        .current_dir(root)
        .status()
        .expect("run uv to generate the synthetic sets");
    if !status.success() {
        // Do not leave a half-written folder behind.
        let _ = std::fs::remove_dir_all(&tmp);
        panic!("make_sets.py failed: {status}");
    }
    std::fs::write(tmp.join(COMPLETE_MARKER), "ok").expect("write the completion marker");
    if let Err(err) = std::fs::rename(&tmp, dir) {
        // Another test binary may have finished first; use its folder if it is complete.
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(
            dir.join(COMPLETE_MARKER).is_file(),
            "rename {} failed: {err}",
            tmp.display()
        );
    }
}
