//! Plain-language error messages for the people using the app.

use crate::download::DownloadError;
use puw_core::Error;
use serde::Serialize;
use std::io::ErrorKind;

/// Everything that can go wrong in the app.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Core(#[from] Error),
    #[error("no writable output folder")]
    NoWritableOutput,
    #[error("nothing to remove")]
    NothingToRemove,
    #[error("not ready")]
    NotReady,
    #[error("busy")]
    Busy,
    #[error(transparent)]
    Download(#[from] DownloadError),
}

/// What the interface shows: a short message and the raw text for "Show details".
#[derive(Debug, Clone, Serialize)]
pub struct UserError {
    pub message: String,
    pub details: String,
}

impl From<AppError> for UserError {
    fn from(err: AppError) -> UserError {
        UserError {
            message: user_message(&err),
            details: err.to_string(),
        }
    }
}

/// A sentence a non-technical person can act on.
pub fn user_message(err: &AppError) -> String {
    let text = match err {
        AppError::Core(e) => return core_message(e),
        AppError::NoWritableOutput => {
            "There is no folder the app can save photos to. Choose another folder in Settings."
        }
        AppError::NothingToRemove => {
            "No watermark area has been chosen yet. Choose where the watermark is first."
        }
        AppError::NotReady => {
            "The watermark remover is not ready yet. Please wait a moment and try again."
        }
        AppError::Busy => "The app is still working on your photos. Please wait until it finishes.",
        AppError::Download(e) => return download_message(e),
    };
    text.to_string()
}

fn download_message(e: &DownloadError) -> String {
    match e {
        DownloadError::Network(_) => {
            "The download was interrupted. Check the internet connection and try again."
        }
        DownloadError::Verify { .. } => {
            "The downloaded file arrived damaged. Please try the download again."
        }
        DownloadError::Disk(_) => {
            "The download could not be saved. Check that the disk has enough free space and try again."
        }
        DownloadError::Cancelled => "Download stopped. It will carry on where it left off next time.",
        DownloadError::BadName(_) => {
            "The download list is not valid. Please install the latest version of the app."
        }
    }
    .to_string()
}

fn core_message(e: &Error) -> String {
    match e {
        Error::Cancelled => "Stopped.".into(),
        Error::UnsupportedFormat(f) if f.eq_ignore_ascii_case("heic") => {
            "HEIC photos are not supported yet. Export them as JPEG from your photo app, then add them again.".into()
        }
        Error::UnsupportedFormat(_) => {
            "This kind of file is not supported. Use JPEG, PNG, TIFF or WebP photos.".into()
        }
        Error::Io(io) => match io.kind() {
            ErrorKind::NotFound => {
                "The photo could not be found. It may have been moved or deleted.".into()
            }
            ErrorKind::PermissionDenied => {
                "The app is not allowed to write to this folder.".into()
            }
            _ => "The app could not read or save the photo. Check that the disk is not full or disconnected.".into(),
        },
        Error::Image(_) => "The photo could not be opened. The file may be damaged.".into(),
        Error::Incomplete(_) => "The photo file is incomplete or damaged.".into(),
        Error::InvalidInput(_) => {
            "That name cannot be used. Choose a different name and try again.".into()
        }
        Error::Model(_) => "The watermark remover could not run on this photo.".into(),
        Error::SizeMismatch => {
            "These photos are not all the same size, so they cannot be handled together.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn all() -> Vec<AppError> {
        let io_kinds = [
            ErrorKind::NotFound,
            ErrorKind::PermissionDenied,
            ErrorKind::Other,
        ];
        let mut v: Vec<AppError> = io_kinds
            .into_iter()
            .map(|k| Error::Io(io::Error::new(k, "x")).into())
            .collect();
        v.extend([
            Error::Image("x".into()).into(),
            Error::Incomplete("x".into()).into(),
            Error::InvalidInput("x".into()).into(),
            Error::Model("x".into()).into(),
            Error::SizeMismatch.into(),
            Error::UnsupportedFormat("heic".into()).into(),
            Error::UnsupportedFormat("bmp".into()).into(),
            Error::Cancelled.into(),
            AppError::NoWritableOutput,
            AppError::NothingToRemove,
            AppError::NotReady,
            AppError::Busy,
            DownloadError::Network("x".into()).into(),
            DownloadError::Verify { name: "x".into() }.into(),
            DownloadError::Disk("x".into()).into(),
            DownloadError::Cancelled.into(),
            DownloadError::BadName("x".into()).into(),
        ]);
        v
    }

    #[test]
    fn messages_are_plain() {
        let banned_words = ["mask", "mark", "unmark"];
        let banned_parts = ["inpaint", "segment", "threshold", "dilat", "onnx", "tensor"];
        let rust_names = ["Error", "Err(", "Io(", "::", "{", "}"];
        for e in all() {
            let m = user_message(&e);
            assert!(!m.trim().is_empty(), "{e:?}");
            let lower = m.to_lowercase();
            for w in lower.split(|c: char| !c.is_alphanumeric()) {
                assert!(!banned_words.contains(&w), "{e:?}: {m}");
            }
            for p in banned_parts {
                assert!(!lower.contains(p), "{e:?}: {m}");
            }
            for r in rust_names {
                assert!(!m.contains(r), "{e:?}: {m}");
            }
            assert!(m.is_ascii(), "{e:?}: {m}");
            assert!(!m.contains('\u{2014}') && !m.contains('\u{2013}'));
        }
    }

    #[test]
    fn invalid_input_is_not_image_error() {
        let m = user_message(&Error::InvalidInput("empty name".into()).into());
        assert!(m.to_lowercase().contains("name"));
        assert!(!m.to_lowercase().contains("photo"));
    }

    #[test]
    fn specific_messages() {
        let m = |e: Error| user_message(&e.into());
        assert_eq!(m(Error::Cancelled), "Stopped.");
        assert!(m(Error::UnsupportedFormat("heic".into())).contains("HEIC"));
        assert!(m(Error::Io(io::ErrorKind::NotFound.into())).contains("moved or deleted"));
        assert!(m(Error::Io(io::ErrorKind::PermissionDenied.into())).contains("not allowed"));
        assert_eq!(
            m(Error::Model("x".into())),
            "The watermark remover could not run on this photo."
        );
    }

    #[test]
    fn download_messages_are_specific() {
        let m = |e: DownloadError| user_message(&e.into());
        assert_eq!(
            m(DownloadError::Network("dns".into())),
            "The download was interrupted. Check the internet connection and try again."
        );
        assert!(m(DownloadError::Disk("x".into())).contains("free space"));
        assert!(m(DownloadError::Cancelled).contains("stopped"));
    }

    #[test]
    fn user_error_carries_details() {
        let u: UserError = AppError::Core(Error::Model("boom".into())).into();
        assert!(u.details.contains("boom"));
        assert!(!u.message.contains("boom"));
    }
}
