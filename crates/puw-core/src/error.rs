//! Error type shared by the whole crate.

/// Everything that can go wrong in `puw-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("image error: {0}")]
    Image(String),
    /// The file ends before the photo does (a copy or download that was cut short).
    #[error("incomplete file: {0}")]
    Incomplete(String),
    /// A value passed in by the caller is not acceptable (an empty name, a malformed id).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("model error: {0}")]
    Model(String),
    #[error("the images do not all have the same size")]
    SizeMismatch,
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("cancelled")]
    Cancelled,
}

/// `Result` with [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
