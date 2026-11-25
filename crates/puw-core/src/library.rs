//! Saved watermarks: one JSON file per watermark in a per-user folder, plus suggestion of a
//! saved watermark for a new set of photos.
//!
//! File format: `<dir>/<id>.json` holds the whole [`SavedWatermark`]. `thumbnail_png` is stored
//! as a JSON array of byte values (serde's default for `Vec<u8>`), which avoids another crate.

use crate::template::{MatchStatus, WatermarkTemplate, find};
use crate::{Error, Result};
use image::RgbImage;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

/// Number of sample photos `suggest` looks at.
const SUGGEST_PHOTOS: usize = 5;

/// A watermark the user chose to keep for later sets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedWatermark {
    /// UUID v4, also the file stem.
    pub id: String,
    pub name: String,
    /// RFC 3339, UTC.
    pub created: String,
    pub template: WatermarkTemplate,
    pub thumbnail_png: Vec<u8>,
}

impl SavedWatermark {
    /// A new entry with a fresh id and the current time.
    pub fn new(name: &str, template: WatermarkTemplate, thumbnail_png: Vec<u8>) -> SavedWatermark {
        let created = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
        SavedWatermark {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            created,
            template,
            thumbnail_png,
        }
    }
}

/// A folder of saved watermarks.
#[derive(Debug, Clone)]
pub struct Library {
    dir: PathBuf,
}

fn invalid(msg: &str) -> Error {
    Error::Image(msg.to_string())
}

fn invalid_input(msg: &str) -> Error {
    Error::InvalidInput(msg.to_string())
}

fn not_found() -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no such saved watermark",
    ))
}

/// Accept only canonical lower-case hyphenated UUIDs, so an id can never leave the folder.
fn check_id(id: &str) -> Result<()> {
    match Uuid::try_parse(id) {
        Ok(u) if u.hyphenated().to_string() == id => Ok(()),
        _ => Err(invalid_input("invalid watermark id")),
    }
}

fn clean_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        Err(invalid_input("empty name"))
    } else {
        Ok(n.to_string())
    }
}

impl Library {
    /// Open the library in `dir`, creating the folder if needed.
    pub fn open(dir: &Path) -> Result<Library> {
        std::fs::create_dir_all(dir)?;
        Ok(Library {
            dir: dir.to_path_buf(),
        })
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn read(&self, id: &str) -> Result<SavedWatermark> {
        check_id(id)?;
        let bytes = std::fs::read(self.path(id))?;
        serde_json::from_slice(&bytes).map_err(|e| invalid(&format!("bad library file: {e}")))
    }

    /// All readable saved watermarks, newest first (ties by id). Unreadable files are skipped.
    pub fn list(&self) -> Result<Vec<SavedWatermark>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Ok(w) = self.read(stem)
                && w.id == stem
            {
                out.push(w);
            }
        }
        out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.id.cmp(&b.id)));
        Ok(out)
    }

    /// Write `w` to `<id>.json` atomically (temp file in the same folder, fsync, rename).
    pub fn save(&self, w: &SavedWatermark) -> Result<()> {
        check_id(&w.id)?;
        let mut w = w.clone();
        w.name = clean_name(&w.name)?;
        self.write(&w)
    }

    fn write(&self, w: &SavedWatermark) -> Result<()> {
        let json = serde_json::to_vec(w).map_err(|e| invalid(&format!("cannot encode: {e}")))?;
        let tmp = self.dir.join(format!("{}.json.tmp", w.id));
        let result = (|| -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
            std::fs::rename(&tmp, self.path(&w.id))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        Ok(result?)
    }

    /// Change the name of a saved watermark (whitespace trimmed, must not end up empty).
    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        check_id(id)?;
        let name = clean_name(name)?;
        let mut w = self.read(id).map_err(|e| match e {
            Error::Io(ref io) if io.kind() == std::io::ErrorKind::NotFound => not_found(),
            other => other,
        })?;
        w.name = name;
        self.write(&w)
    }

    /// Remove a saved watermark.
    pub fn delete(&self, id: &str) -> Result<()> {
        check_id(id)?;
        match std::fs::remove_file(self.path(id)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(not_found()),
            r => Ok(r?),
        }
    }

    /// Look for a saved watermark in the first few `sample` photos. A watermark qualifies if at
    /// least half of the photos tried are `Found`; the best mean confidence wins.
    pub fn suggest(&self, sample: &[&RgbImage]) -> Result<Option<(String, f32)>> {
        self.suggest_until(sample, &AtomicBool::new(false))
    }

    /// [`Library::suggest`] that stops with [`Error::Cancelled`] once `stop` is set (checked
    /// before each photo is searched).
    pub fn suggest_until(
        &self,
        sample: &[&RgbImage],
        stop: &AtomicBool,
    ) -> Result<Option<(String, f32)>> {
        let photos = &sample[..sample.len().min(SUGGEST_PHOTOS)];
        if photos.is_empty() {
            return Ok(None);
        }
        let need = photos.len().div_ceil(2);
        let mut best: Option<(String, f32)> = None;
        for w in self.list()? {
            let (found, mean) = score(&w.template, photos, stop)?;
            if found >= need && best.as_ref().is_none_or(|(_, b)| mean > *b) {
                best = Some((w.id, mean));
            }
        }
        Ok(best)
    }
}

/// Number of `Found` photos and the mean confidence over all photos.
fn score(t: &WatermarkTemplate, photos: &[&RgbImage], stop: &AtomicBool) -> Result<(usize, f32)> {
    let mut found = 0;
    let mut sum = 0.0;
    for img in photos {
        if stop.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let r = find(t, img);
        found += (r.status == MatchStatus::Found) as usize;
        sum += r.confidence;
    }
    Ok((found, sum / photos.len() as f32))
}
