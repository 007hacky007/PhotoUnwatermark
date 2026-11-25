//! Where the app keeps its files.

use crate::Result;
use puw_core::Error;
use std::path::{Path, PathBuf};

/// The app's data folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDirs {
    pub root: PathBuf,
    pub models: PathBuf,
    pub watermarks: PathBuf,
    /// The settings file.
    pub settings: PathBuf,
}

impl DataDirs {
    /// The per-user data folder for this platform.
    pub fn default_location() -> Result<DataDirs> {
        let base = dirs::data_local_dir().ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no user data folder",
            ))
        })?;
        Ok(DataDirs::at(&base.join("PhotoUnwatermark")))
    }

    /// Folders under an explicit root (used by tests).
    pub fn at(root: &Path) -> DataDirs {
        DataDirs {
            root: root.to_path_buf(),
            models: root.join("models"),
            watermarks: root.join("watermarks"),
            settings: root.join("settings.json"),
        }
    }

    /// Create the folders that do not exist yet.
    pub fn ensure(&self) -> Result<()> {
        for dir in [&self.root, &self.models, &self.watermarks] {
            std::fs::create_dir_all(dir).map_err(Error::Io)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_lays_out_folders() {
        let d = DataDirs::at(Path::new("/x"));
        assert_eq!(d.models, Path::new("/x/models"));
        assert_eq!(d.watermarks, Path::new("/x/watermarks"));
        assert_eq!(d.settings, Path::new("/x/settings.json"));
    }

    #[test]
    fn ensure_creates_folders() {
        let t = tempfile::tempdir().unwrap();
        let d = DataDirs::at(&t.path().join("data"));
        d.ensure().unwrap();
        assert!(d.models.is_dir() && d.watermarks.is_dir());
        d.ensure().unwrap();
    }

    #[test]
    fn default_location_is_named_for_the_app() {
        let d = DataDirs::default_location().unwrap();
        assert_eq!(
            d.root,
            dirs::data_local_dir().unwrap().join("PhotoUnwatermark")
        );
    }
}
