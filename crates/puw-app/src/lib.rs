//! App logic for PhotoUnwatermark, free of any UI toolkit.

pub mod download;
pub mod estimate;
pub mod images;
pub mod messages;
pub mod paths;
pub mod photos;
pub mod settings;

pub use estimate::{Eta, load_fraction};
pub use messages::{AppError, UserError, user_message};
pub use paths::DataDirs;
pub use settings::{Detail, OutputRule, Settings};

/// `Result` with [`AppError`].
pub type Result<T> = std::result::Result<T, AppError>;
