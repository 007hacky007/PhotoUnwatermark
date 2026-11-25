//! User settings, stored as JSON.

use crate::Result;
use puw_core::{Error, ModelKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where finished photos go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OutputRule {
    NextToOriginals { folder_name: String },
    Folder { path: PathBuf },
}

impl Default for OutputRule {
    fn default() -> Self {
        OutputRule::NextToOriginals {
            folder_name: "Without watermark".into(),
        }
    }
}

/// How much detail to keep while working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Detail {
    #[default]
    Best,
    Balanced,
    Faster,
}

impl Detail {
    /// Longest side, in pixels, that photos are worked at.
    pub fn work_long_side(self) -> u32 {
        match self {
            Detail::Best => 4096,
            Detail::Balanced => 3072,
            Detail::Faster => 2048,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub output: OutputRule,
    pub detail: Detail,
    pub prefer_gpu: bool,
    /// Measured seconds to load each model, used for progress estimates.
    pub load_seconds: BTreeMap<ModelKind, f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            output: OutputRule::default(),
            detail: Detail::default(),
            prefer_gpu: true,
            load_seconds: BTreeMap::new(),
        }
    }
}

/// Whether `name` can be the name of the folder made next to the originals on every system:
/// not empty, not `.` or `..`, no path separator, none of the characters Windows forbids
/// (`< > : " | ? *` and control characters), no space or dot at the end and not a name
/// Windows keeps for devices (`CON`, `NUL`, `COM1`, ... also with an extension).
pub fn valid_folder_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.ends_with(['.', ' ']) {
        return false;
    }
    if name.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
    }) {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let numbered = |p: &str| {
        stem.len() == 4 && stem.starts_with(p) && matches!(stem.as_bytes()[3], b'1'..=b'9')
    };
    !(matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT"))
}

impl Settings {
    /// Read settings; a missing or unreadable file gives the defaults.
    pub fn load(path: &Path) -> Settings {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Write settings to a temp file next to `path`, then rename it into place.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json =
            serde_json::to_vec_pretty(self).map_err(|e| Error::InvalidInput(e.to_string()))?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let written = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(Error::Io(e).into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults() {
        let s = Settings::default();
        assert_eq!(
            s.output,
            OutputRule::NextToOriginals {
                folder_name: "Without watermark".into()
            }
        );
        assert_eq!(s.detail, Detail::Best);
        assert!(s.prefer_gpu);
        assert!(s.load_seconds.is_empty());
    }

    #[test]
    fn settings_roundtrip() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        let mut s = Settings {
            output: OutputRule::Folder {
                path: "/photos/out".into(),
            },
            detail: Detail::Faster,
            prefer_gpu: false,
            ..Settings::default()
        };
        s.load_seconds.insert(ModelKind::Lama, 4.5);
        s.load_seconds.insert(ModelKind::Face, 0.5);
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);
    }

    #[test]
    fn settings_use_camel_case() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains("preferGpu") && json.contains("loadSeconds"));
        assert!(json.contains("nextToOriginals") && json.contains("folderName"));
    }

    #[test]
    fn settings_missing_file_gives_defaults() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(
            Settings::load(&t.path().join("none.json")),
            Settings::default()
        );
    }

    #[test]
    fn settings_garbage_gives_defaults() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        std::fs::write(&p, b"{ not json \xff\xfe").unwrap();
        assert_eq!(Settings::load(&p), Settings::default());
    }

    #[test]
    fn settings_partial_file_fills_defaults() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        std::fs::write(&p, br#"{"detail":"faster"}"#).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.detail, Detail::Faster);
        assert!(s.prefer_gpu);
    }

    #[test]
    fn settings_save_is_atomic() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        Settings::default().save(&p).unwrap();
        Settings::default().save(&p).unwrap();
        let names: Vec<_> = std::fs::read_dir(t.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["settings.json".to_string()]);
    }

    #[test]
    fn settings_save_into_missing_folder_fails_cleanly() {
        let t = tempfile::tempdir().unwrap();
        assert!(
            Settings::default()
                .save(&t.path().join("no/such/settings.json"))
                .is_err()
        );
    }

    #[test]
    fn folder_names() {
        for ok in [
            "Without watermark",
            "Out",
            "a.b",
            "2026-11",
            ".hidden",
            "Fotky bez znaku",
        ] {
            assert!(valid_folder_name(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "x:",
            "a*",
            "q?",
            "<x>",
            "\"q\"",
            "p|q",
            "end.",
            "end ",
            "CON",
            "con.txt",
            "NUL",
            "COM1",
            "lpt9.x",
            "tab\there",
        ] {
            assert!(!valid_folder_name(bad), "{bad:?}");
        }
        assert!(valid_folder_name("COM0") && valid_folder_name("CONSOLE"));
    }

    #[test]
    fn detail_sides() {
        assert_eq!(Detail::Best.work_long_side(), 4096);
        assert_eq!(Detail::Balanced.work_long_side(), 3072);
        assert_eq!(Detail::Faster.work_long_side(), 2048);
    }
}
