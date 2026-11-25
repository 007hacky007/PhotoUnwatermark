//! Images the interface shows, kept in memory: small thumbnails and the larger edit images as
//! JPEG, and the grid thumbnails as pixels (the current area is drawn on them per request).

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use image::RgbImage;
use puw_core::load;

use crate::Result;
use crate::images::{self, GRID_LONG_SIDE, THUMB_LONG_SIDE, jpeg};
use crate::photos::PhotoId;

/// Thumbnails kept in memory.
const THUMB_CACHE: usize = 512;
/// Edit images kept in memory.
const EDIT_CACHE: usize = 4;
/// Grid thumbnails kept in memory (at most 480x480 RGB, so at most about 180 MB).
const GRID_CACHE: usize = 256;
const THUMB_QUALITY: u8 = 85;
const EDIT_QUALITY: u8 = 90;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The thumbnail, edit-image and grid caches. Each lock is held only to look up or store an
/// image, never while a photo is read or encoded.
pub(crate) struct ImageCache {
    thumbs: Mutex<Lru<Vec<u8>>>,
    edits: Mutex<Lru<Vec<u8>>>,
    grids: Mutex<Lru<RgbImage>>,
}

impl ImageCache {
    pub fn new() -> ImageCache {
        ImageCache {
            thumbs: Mutex::new(Lru::new(THUMB_CACHE)),
            edits: Mutex::new(Lru::new(EDIT_CACHE)),
            grids: Mutex::new(Lru::new(GRID_CACHE)),
        }
    }

    /// Small JPEG of photo `id` at `path` (long side [`THUMB_LONG_SIDE`]).
    pub fn thumb(&self, id: PhotoId, path: &Path) -> Result<Vec<u8>> {
        let bytes = cached(&self.thumbs, id, || {
            Ok(jpeg(
                &images::thumbnail(path, THUMB_LONG_SIDE)?,
                THUMB_QUALITY,
            ))
        })?;
        Ok(bytes.to_vec())
    }

    /// JPEG of photo `id` at `path` at edit size.
    pub fn edit(&self, id: PhotoId, path: &Path) -> Result<Vec<u8>> {
        let bytes = cached(&self.edits, id, || {
            Ok(jpeg(&images::edit_image(&load(path)?), EDIT_QUALITY))
        })?;
        Ok(bytes.to_vec())
    }

    /// Photo `id` at `path` for the grid (long side [`GRID_LONG_SIDE`]), without any area.
    pub fn grid(&self, id: PhotoId, path: &Path) -> Result<Arc<RgbImage>> {
        cached(&self.grids, id, || images::thumbnail(path, GRID_LONG_SIDE))
    }

    pub fn remove(&self, id: PhotoId) {
        lock(&self.thumbs).remove(id);
        lock(&self.edits).remove(id);
        lock(&self.grids).remove(id);
    }

    pub fn clear(&self) {
        lock(&self.thumbs).clear();
        lock(&self.edits).clear();
        lock(&self.grids).clear();
    }
}

fn cached<T>(
    cache: &Mutex<Lru<T>>,
    id: PhotoId,
    make: impl FnOnce() -> Result<T>,
) -> Result<Arc<T>> {
    if let Some(hit) = lock(cache).get(id) {
        return Ok(hit);
    }
    let item = Arc::new(make()?);
    lock(cache).put(id, item.clone());
    Ok(item)
}

/// The most recently used images, by photo.
struct Lru<T> {
    cap: usize,
    items: VecDeque<(PhotoId, Arc<T>)>,
}

impl<T> Lru<T> {
    fn new(cap: usize) -> Lru<T> {
        Lru {
            cap,
            items: VecDeque::new(),
        }
    }

    fn get(&mut self, id: PhotoId) -> Option<Arc<T>> {
        let i = self.items.iter().position(|(k, _)| *k == id)?;
        let item = self.items.remove(i)?;
        let value = item.1.clone();
        self.items.push_back(item);
        Some(value)
    }

    fn put(&mut self, id: PhotoId, value: Arc<T>) {
        self.remove(id);
        self.items.push_back((id, value));
        while self.items.len() > self.cap {
            self.items.pop_front();
        }
    }

    fn remove(&mut self, id: PhotoId) {
        self.items.retain(|(k, _)| *k != id);
    }

    fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keeps_most_recently_used() {
        let mut c = Lru::new(2);
        let b = |v: u8| Arc::new(vec![v]);
        c.put(1, b(1));
        c.put(2, b(2));
        assert!(c.get(1).is_some());
        c.put(3, b(3));
        assert!(c.get(2).is_none());
        assert_eq!(c.get(1).unwrap().as_slice(), &[1]);
        assert_eq!(c.get(3).unwrap().as_slice(), &[3]);
        c.put(3, b(4));
        assert_eq!(c.items.len(), 2);
        assert_eq!(c.get(3).unwrap().as_slice(), &[4]);
    }
}
