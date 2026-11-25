pub mod batch;
pub mod detect;
pub mod engine;
mod error;
pub mod faces;
pub mod image_io;
pub mod inpaint;
pub mod library;
pub mod mask;
pub mod ncc;
pub mod pipeline;
pub mod template;
pub mod tiles;

pub use detect::{Guess, GuessOptions};
pub use engine::{Backend, CpuReason, DeviceReport, Engine, EngineOptions, ModelKind, ModelPaths};
pub use error::{Error, Result};
pub use image_io::{LoadedPhoto, load, output_dir_for};
pub use inpaint::{Progress, WORK_LONG_SIDE};
pub use library::{Library, SavedWatermark};
pub use mask::Mask;
pub use pipeline::{
    SetLayers, build_set_layers, guess_photo, match_photo, pick_representative, remove_one,
    representative_sample,
};
pub use template::{
    MatchBasis, MatchResult, MatchStatus, Transform, WatermarkTemplate, build_template, find,
};
