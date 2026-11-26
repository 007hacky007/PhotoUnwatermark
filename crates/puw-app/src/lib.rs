//! App logic for PhotoUnwatermark, free of any UI toolkit.

pub mod core;
pub mod device;
pub mod download;
pub mod estimate;
mod image_cache;
pub mod images;
pub mod jobs;
pub mod messages;
pub mod output;
pub mod paths;
pub mod photos;
mod session;
pub mod settings;

pub use crate::core::{
    AppCore, AreaStatus, DetectOutcome, Done, Failed, GroupInfo, GroupsState, LeftOut,
    MatchSummary, OutputCheck, PREVIEW_LONG_SIDE, PhotoState, Placed, RemoveReport, SavedInfo,
    StripPlacement, Suggestion, default_watermark_name,
};
pub use device::{DeviceFix, DeviceState, DeviceStatus, Platform, device_status};
pub use estimate::{Eta, load_fraction};
pub use jobs::{Events, LoadPurpose, ProgressEvent, with_ticket};
pub use messages::{AppError, UserError, user_message};
pub use paths::DataDirs;
pub use settings::{Detail, OutputRule, Settings};

/// `Result` with [`AppError`].
pub type Result<T> = std::result::Result<T, AppError>;
