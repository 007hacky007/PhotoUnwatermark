//! The list of photos the person has added, and how files and folders become entries.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use puw_core::Error;
use puw_core::image_io::upright_size;
use rayon::prelude::*;
use serde::Serialize;

use crate::OutputRule;
use crate::messages::{AppError, user_message};

pub type PhotoId = u32;

/// Why a photo in the list cannot be used: it was found, when it was read, to be gone or
/// impossible to open (its header read fine when it was added).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Problem {
    /// The file is no longer there (moved or deleted after it was added).
    Gone,
    /// The file is there but cannot be opened (damaged, cut short, not readable).
    Damaged,
}

impl Problem {
    /// The problem a failed read of a photo shows, or `None` for an error that says nothing
    /// about the file (stopped, a model that failed, ...).
    pub fn of(e: &AppError) -> Option<Problem> {
        match e {
            AppError::Core(Error::Io(io)) if io.kind() == std::io::ErrorKind::NotFound => {
                Some(Problem::Gone)
            }
            AppError::Core(
                Error::Io(_) | Error::Image(_) | Error::Incomplete(_) | Error::UnsupportedFormat(_),
            ) => Some(Problem::Damaged),
            _ => None,
        }
    }

    /// The sentence saying what is wrong with the photo.
    pub fn message(self) -> String {
        let e = match self {
            Problem::Gone => Error::Io(std::io::ErrorKind::NotFound.into()),
            Problem::Damaged => Error::Image(String::new()),
        };
        user_message(&AppError::Core(e))
    }
}

/// What the interface shows about a photo. Width and height are the upright size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoInfo {
    pub id: PhotoId,
    pub name: String,
    pub folder: String,
    pub width: u32,
    pub height: u32,
    /// Set once reading the photo failed; such a photo is never worked on.
    pub problem: Option<Problem>,
}

#[derive(Debug, Clone)]
pub struct PhotoEntry {
    pub info: PhotoInfo,
    pub path: PathBuf,
}

/// A file that was not added, with a sentence saying why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddResult {
    pub added: Vec<PhotoInfo>,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone, Default)]
pub struct PhotoList {
    entries: Vec<PhotoEntry>,
    next_id: PhotoId,
}

const EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "tif", "tiff", "webp"];

fn has_supported_extension(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

fn same_name(a: &str, b: &str) -> bool {
    if cfg!(any(target_os = "macos", windows)) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

/// `p` without the `\\?\` prefix Windows puts on canonical paths (`C:\a` instead of
/// `\\?\C:\a`, `\\server\share` instead of `\\?\UNC\server\share`), for paths that are
/// shown or compared with paths the person chose. Paths the prefix is needed for (too long,
/// or with names Windows does not allow) are kept as they are. Pure text handling, so it
/// behaves the same on every system.
pub fn plain_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    let plain = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        let drive = rest.as_bytes();
        if drive.len() >= 3 && drive[0].is_ascii_alphabetic() && &drive[1..3] == b":\\" {
            rest.to_string()
        } else {
            return p.to_path_buf();
        }
    } else {
        return p.to_path_buf();
    };
    // Longer paths, and names such as "CON" or "a.", only work with the prefix.
    let reserved = |c: &str| {
        let stem = c.split('.').next().unwrap_or("").to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
            || c.ends_with('.')
            || c.ends_with(' ')
    };
    if plain.len() >= 260 || plain.split('\\').skip(1).any(reserved) {
        return p.to_path_buf();
    }
    PathBuf::from(plain)
}

/// The real path of `p` (links followed), without the Windows `\\?\` prefix where it is
/// not needed ([`plain_path`]), so it compares with and shows like the paths people choose.
pub fn real_path(p: &Path) -> std::io::Result<PathBuf> {
    let real = fs::canonicalize(p)?;
    Ok(if cfg!(windows) {
        plain_path(&real)
    } else {
        real
    })
}

/// True when `p` is `dir` or inside it. Both are real paths ([`real_path`]); names compare
/// the way the file system does (without case on macOS and Windows).
fn within(p: &Path, dir: &Path) -> bool {
    let mut p = p.components();
    dir.components().all(|d| {
        p.next().is_some_and(|c| {
            same_name(
                &c.as_os_str().to_string_lossy(),
                &d.as_os_str().to_string_lossy(),
            )
        })
    })
}

/// Where finished photos are saved, as far as adding photos is concerned: whatever is in
/// there is a result of an earlier run and is left out (with a reason).
enum Results {
    /// A folder of this name next to the originals: any folder of this name.
    Named(String),
    /// One chosen folder (real path), or `None` when it does not exist yet.
    Chosen(Option<PathBuf>),
}

impl Results {
    fn of(rule: &OutputRule) -> Results {
        match rule {
            OutputRule::NextToOriginals { folder_name } => Results::Named(folder_name.clone()),
            OutputRule::Folder { path } => Results::Chosen(real_path(path).ok()),
        }
    }

    /// Why the file or folder at real path `p` is left out, or `None` when it is not.
    fn reason(&self, p: &Path, is_dir: bool) -> Option<String> {
        match self {
            Results::Named(name) => {
                let inside = p.components().any(|c| match c {
                    std::path::Component::Normal(n) => same_name(&n.to_string_lossy(), name),
                    _ => false,
                });
                inside.then(|| {
                    if is_dir {
                        format!(
                            "This folder was left out: it is named \"{name}\" or is inside a folder of that name, where finished photos are saved."
                        )
                    } else {
                        format!(
                            "This photo was left out: it is inside a folder named \"{name}\", where finished photos are saved."
                        )
                    }
                })
            }
            Results::Chosen(Some(dir)) if within(p, dir) => Some(if is_dir {
                "This folder was left out: finished photos are saved there.".into()
            } else {
                "This photo was left out: it is in the folder where finished photos are saved."
                    .into()
            }),
            Results::Chosen(_) => None,
        }
    }
}

impl PhotoList {
    /// Add files and folders (one level deep). Photos already in the list are left out
    /// quietly. Results of earlier runs are left out with a reason in
    /// [`AddResult::skipped`]: with [`OutputRule::Folder`] whatever is inside the chosen folder,
    /// with [`OutputRule::NextToOriginals`] whatever is inside a folder of that name.
    pub fn add_paths(&mut self, paths: &[PathBuf], rule: &OutputRule) -> AddResult {
        let results = Results::of(rule);
        let left_out = |p: &Path, is_dir: bool| {
            real_path(p)
                .ok()
                .and_then(|r| results.reason(&r, is_dir))
                .map(|reason| Skipped {
                    name: file_name(p),
                    reason,
                })
        };
        let mut result = AddResult::default();
        let mut candidates: Vec<PathBuf> = Vec::new();
        for p in paths {
            if p.is_dir() {
                if let Some(s) = left_out(p, true) {
                    result.skipped.push(s);
                    continue;
                }
                let Ok(rd) = fs::read_dir(p) else {
                    result.skipped.push(Skipped {
                        name: file_name(p),
                        reason: "This folder could not be opened.".into(),
                    });
                    continue;
                };
                let mut files: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|f| f.is_file() && has_supported_extension(f))
                    .filter(|f| !is_hidden(f))
                    .collect();
                files.sort();
                candidates.extend(files);
            } else if let Some(s) = left_out(p, false) {
                result.skipped.push(s);
            } else if !has_supported_extension(p) {
                result.skipped.push(Skipped {
                    name: file_name(p),
                    reason: if is_heic_name(p) {
                        heic_reason()
                    } else {
                        "This kind of file is not supported. Use JPEG, PNG, TIFF or WebP photos."
                            .into()
                    },
                });
            } else {
                candidates.push(p.clone());
            }
        }

        let mut seen: HashSet<PathBuf> = self.entries.iter().map(|e| e.path.clone()).collect();
        let mut fresh: Vec<PathBuf> = Vec::new();
        for c in candidates {
            match real_path(&c) {
                Ok(real) => {
                    if let Some(reason) = results.reason(&real, false) {
                        result.skipped.push(Skipped {
                            name: file_name(&c),
                            reason,
                        });
                        continue;
                    }
                    if seen.insert(real.clone()) {
                        fresh.push(real);
                    }
                }
                Err(_) => result.skipped.push(Skipped {
                    name: file_name(&c),
                    reason: "The photo could not be found. It may have been moved or deleted."
                        .into(),
                }),
            }
        }

        let sizes = read_sizes(&fresh);
        for (path, size) in fresh.into_iter().zip(sizes) {
            match size {
                Ok((width, height)) => {
                    let info = PhotoInfo {
                        id: self.next_id,
                        name: file_name(&path),
                        folder: path.parent().map(file_name).unwrap_or_default(),
                        width,
                        height,
                        problem: None,
                    };
                    self.next_id += 1;
                    result.added.push(info.clone());
                    self.entries.push(PhotoEntry { info, path });
                }
                Err(e) => result.skipped.push(Skipped {
                    name: file_name(&path),
                    reason: reason_for(e),
                }),
            }
        }
        result
    }

    pub fn remove(&mut self, id: PhotoId) {
        self.entries.retain(|e| e.info.id != id);
    }

    /// Notes that photo `id` turned out to be gone or impossible to open.
    pub fn set_problem(&mut self, id: PhotoId, problem: Problem) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.info.id == id) {
            e.info.problem = Some(problem);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn get(&self, id: PhotoId) -> Option<&PhotoEntry> {
        self.entries.iter().find(|e| e.info.id == id)
    }

    pub fn infos(&self) -> Vec<PhotoInfo> {
        self.entries.iter().map(|e| e.info.clone()).collect()
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.entries.iter().map(|e| e.path.clone()).collect()
    }
}

fn is_hidden(p: &Path) -> bool {
    p.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn is_heic_name(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
}

fn heic_reason() -> String {
    user_message(&AppError::Core(Error::UnsupportedFormat("heic".into())))
}

fn reason_for(e: Error) -> String {
    match e {
        Error::UnsupportedFormat(_) => user_message(&AppError::Core(e)),
        _ => "This photo could not be read. The file may be damaged.".into(),
    }
}

fn read_sizes(paths: &[PathBuf]) -> Vec<puw_core::Result<(u32, u32)>> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        / 2;
    match rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()
    {
        Ok(pool) => pool.install(|| paths.par_iter().map(|p| upright_size(p)).collect()),
        Err(_) => paths.iter().map(|p| upright_size(p)).collect(),
    }
}
