//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Folder with the model files, from `PUW_MODELS_DIR`. Tests that need models return early
/// with `eprintln!("skipped: PUW_MODELS_DIR not set")` when this is `None`.
pub fn models_dir() -> Option<PathBuf> {
    std::env::var_os("PUW_MODELS_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Compiled-model cache shared by the model tests of this crate, in `<workspace>/target`.
pub fn model_cache_dir() -> PathBuf {
    workspace_root()
        .join("target")
        .join("test-model-cache")
        .join("puw-app")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root above crates/puw-app")
}

/// Synthetic test sets in `<workspace>/target/synth`, generated with tools/synth/make_sets.py
/// when the folder is missing.
///
/// Tests in one binary share a single generation through a `OnceLock`. Separate test binaries
/// may race, so each generates into its own `target/synth.tmp-<pid>` and renames it into place;
/// the loser of a race deletes its copy and uses the folder that won.
pub fn synth_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = workspace_root();
        let dir = root.join("target").join("synth");
        if !dir.exists() {
            generate_into(root, &dir);
        }
        dir
    })
    .clone()
}

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
    if let Err(err) = std::fs::rename(&tmp, dir) {
        // Another test binary finished first; its folder is complete because it was renamed
        // into place only after generation succeeded.
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(dir.is_dir(), "rename {} failed: {err}", tmp.display());
    }
}
