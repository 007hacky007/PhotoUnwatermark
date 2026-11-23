//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

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
/// when the folder is missing.
pub fn synth_dir() -> PathBuf {
    let root = workspace_root();
    let dir = root.join("target").join("synth");
    if !dir.exists() {
        let status = Command::new("uv")
            .args([
                "run",
                "--project",
                "tools",
                "python",
                "tools/synth/make_sets.py",
                "--out",
            ])
            .arg(&dir)
            .current_dir(root)
            .status()
            .expect("run uv to generate the synthetic sets");
        if !status.success() {
            // Do not leave a half-written folder that the next run would trust.
            let _ = std::fs::remove_dir_all(&dir);
            panic!("make_sets.py failed: {status}");
        }
    }
    dir
}
