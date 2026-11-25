//! JPEGs the interface shows, kept in memory: small thumbnails and the larger edit images.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use puw_core::load;

use crate::Result;
use crate::images::{self, THUMB_LONG_SIDE, jpeg};
use crate::photos::PhotoId;

/// Thumbnails kept in memory.
const THUMB_CACHE: usize = 512;
/// Edit images kept in memory.
const EDIT_CACHE: usize = 4;
const THUMB_QUALITY: u8 = 85;
const EDIT_QUALITY: u8 = 90;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The thumbnail and edit-image caches. Each lock is held only to look up or store bytes, never
/// while a photo is read or encoded.
pub(crate) struct ImageCache {
    thumbs: Mutex<JpegCache>,
    edits: Mutex<JpegCache>,
}

impl ImageCache {
    pub fn new() -> ImageCache {
        ImageCache {
            thumbs: Mutex::new(JpegCache::new(THUMB_CACHE)),
            edits: Mutex::new(JpegCache::new(EDIT_CACHE)),
        }
    }

    /// Small JPEG of photo `id` at `path` (long side [`THUMB_LONG_SIDE`]).
    pub fn thumb(&self, id: PhotoId, path: &Path) -> Result<Vec<u8>> {
        cached(&self.thumbs, id, || {
            Ok(jpeg(
                &images::thumbnail(path, THUMB_LONG_SIDE)?,
                THUMB_QUALITY,
            ))
        })
    }

    /// JPEG of photo `id` at `path` at edit size.
    pub fn edit(&self, id: PhotoId, path: &Path) -> Result<Vec<u8>> {
        cached(&self.edits, id, || {
            Ok(jpeg(&images::edit_image(&load(path)?), EDIT_QUALITY))
        })
    }

    pub fn remove(&self, id: PhotoId) {
        lock(&self.thumbs).remove(id);
        lock(&self.edits).remove(id);
    }

    pub fn clear(&self) {
        lock(&self.thumbs).clear();
        lock(&self.edits).clear();
    }
}

fn cached(
    cache: &Mutex<JpegCache>,
    id: PhotoId,
    make: impl FnOnce() -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    if let Some(hit) = lock(cache).get(id) {
        return Ok(hit.to_vec());
    }
    let bytes = Arc::new(make()?);
    lock(cache).put(id, bytes.clone());
    Ok(bytes.to_vec())
}

/// The most recently used JPEGs, by photo.
struct JpegCache {
    cap: usize,
    items: VecDeque<(PhotoId, Arc<Vec<u8>>)>,
}

impl JpegCache {
    fn new(cap: usize) -> JpegCache {
        JpegCache {
            cap,
            items: VecDeque::new(),
        }
    }

    fn get(&mut self, id: PhotoId) -> Option<Arc<Vec<u8>>> {
        let i = self.items.iter().position(|(k, _)| *k == id)?;
        let item = self.items.remove(i)?;
        let bytes = item.1.clone();
        self.items.push_back(item);
        Some(bytes)
    }

    fn put(&mut self, id: PhotoId, bytes: Arc<Vec<u8>>) {
        self.remove(id);
        self.items.push_back((id, bytes));
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
        let mut c = JpegCache::new(2);
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
