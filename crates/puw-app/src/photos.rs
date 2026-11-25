//! The list of photos the person has added, and how files and folders become entries.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use puw_core::Error;
use puw_core::image_io::upright_size;
use rayon::prelude::*;
use serde::Serialize;

use crate::messages::{AppError, user_message};

pub type PhotoId = u32;

/// What the interface shows about a photo. Width and height are the upright size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoInfo {
    pub id: PhotoId,
    pub name: String,
    pub folder: String,
    pub width: u32,
    pub height: u32,
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

fn same_name(name: &str, output_folder_name: &str) -> bool {
    if cfg!(any(target_os = "macos", windows)) {
        name.to_lowercase() == output_folder_name.to_lowercase()
    } else {
        name == output_folder_name
    }
}

/// True when the path, or any folder above it, is the output folder. Meant for canonical paths.
fn inside_output(p: &Path, output_folder_name: &str) -> bool {
    p.components().any(|c| match c {
        std::path::Component::Normal(n) => same_name(&n.to_string_lossy(), output_folder_name),
        _ => false,
    })
}

fn real_inside_output(p: &Path, output_folder_name: &str) -> bool {
    fs::canonicalize(p).is_ok_and(|r| inside_output(&r, output_folder_name))
}

impl PhotoList {
    /// Add files and folders (one level deep). Results of earlier runs (inside the output
    /// folder) and photos already in the list are left out quietly.
    pub fn add_paths(&mut self, paths: &[PathBuf], output_folder_name: &str) -> AddResult {
        let mut result = AddResult::default();
        let mut candidates: Vec<PathBuf> = Vec::new();
        for p in paths {
            if p.is_dir() {
                if real_inside_output(p, output_folder_name) {
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
            } else if real_inside_output(p, output_folder_name) {
                continue;
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
            match fs::canonicalize(&c) {
                Ok(real) => {
                    if inside_output(&real, output_folder_name) {
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
