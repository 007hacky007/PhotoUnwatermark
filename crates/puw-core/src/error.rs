//! Error type shared by the whole crate.

/// Everything that can go wrong in `puw-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("image error: {0}")]
    Image(String),
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
