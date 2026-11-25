//! Where finished photos are saved, and checking that the app can write there.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::OutputRule;

/// The output folder for every folder the `photos` are in: source folder -> output folder.
///
/// Each output folder is created when missing and checked by creating and deleting a small
/// file in it. `Err` lists the source folders (in order) whose output folder cannot be created
/// or written.
pub fn output_dirs(
    photos: &[PathBuf],
    rule: &OutputRule,
) -> Result<BTreeMap<PathBuf, PathBuf>, Vec<PathBuf>> {
    let mut dirs = BTreeMap::new();
    let mut writable: BTreeMap<PathBuf, bool> = BTreeMap::new();
    let mut bad = Vec::new();
    for photo in photos {
        let src = photo.parent().unwrap_or(Path::new("")).to_path_buf();
        if dirs.contains_key(&src) || bad.contains(&src) {
            continue;
        }
        let out = match rule {
            OutputRule::NextToOriginals { folder_name } => src.join(folder_name),
            OutputRule::Folder { path } => path.clone(),
        };
        let ok = *writable
            .entry(out.clone())
            .or_insert_with(|| can_write(&out));
        if ok {
            dirs.insert(src, out);
        } else {
            bad.push(src);
        }
    }
    if bad.is_empty() { Ok(dirs) } else { Err(bad) }
}

/// Creates `dir` if needed, then creates and deletes a file in it.
fn can_write(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    static PROBES: AtomicU32 = AtomicU32::new(0);
    let probe = dir.join(format!(
        ".puw-write-check.{}.{}",
        std::process::id(),
        PROBES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    written && !probe.exists()
}

/// The process id in the name of a file a run leaves while it writes a result
/// (`.<stem>.<pid>.<n>.partial`), or `None` for any other name.
fn partial_pid(name: &str) -> Option<u32> {
    let inner = name.strip_prefix('.')?.strip_suffix(".partial")?;
    let mut parts = inner.rsplitn(3, '.');
    let n = parts.next()?;
    let pid = parts.next()?;
    let stem = parts.next()?;
    if stem.is_empty() || n.parse::<u32>().is_err() {
        return None;
    }
    pid.parse().ok()
}

/// Deletes the half-written results in `dir` that runs of other processes left behind (a run
/// that was killed or crashed). Files of this process and any other file are kept.
pub(crate) fn remove_stale_partials(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let me = std::process::id();
    for e in entries.flatten() {
        let stale = e
            .file_name()
            .to_str()
            .and_then(partial_pid)
            .is_some_and(|pid| pid != me);
        if stale && e.file_type().is_ok_and(|t| t.is_file()) {
            let _ = fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_folder_per_source_folder() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        let photos = [a.join("1.jpg"), b.join("2.jpg"), a.join("3.jpg")];
        let rule = OutputRule::NextToOriginals {
            folder_name: "Out".into(),
        };
        let dirs = output_dirs(&photos, &rule).unwrap();
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[&a], a.join("Out"));
        assert_eq!(dirs[&b], b.join("Out"));
        assert!(a.join("Out").is_dir() && b.join("Out").is_dir());
        // The check leaves nothing behind.
        assert_eq!(fs::read_dir(a.join("Out")).unwrap().count(), 0);

        let chosen = t.path().join("chosen");
        let dirs = output_dirs(
            &photos,
            &OutputRule::Folder {
                path: chosen.clone(),
            },
        )
        .unwrap();
        assert!(dirs.values().all(|d| *d == chosen));
    }

    #[cfg(unix)]
    #[test]
    fn read_only_folders_are_listed() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (ro, rw) = (t.path().join("ro"), t.path().join("rw"));
        fs::create_dir(&ro).unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
        let rule = OutputRule::default();
        let got = output_dirs(
            &[rw.join("1.jpg"), ro.join("2.jpg"), ro.join("3.jpg")],
            &rule,
        );
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(got.unwrap_err(), [ro]);
    }

    #[test]
    fn partial_names() {
        assert_eq!(partial_pid(".a.123.0.partial"), Some(123));
        assert_eq!(partial_pid(".my.photo.7.12.partial"), Some(7));
        assert_eq!(partial_pid("a.123.0.partial"), None);
        assert_eq!(partial_pid(".a.partial"), None);
        assert_eq!(partial_pid(".a.x.0.partial"), None);
        assert_eq!(partial_pid(".a.1.y.partial"), None);
        assert_eq!(partial_pid("..1.0.partial"), None);
        assert_eq!(partial_pid(".a.1.0.part"), None);
    }
}
