//! [`AppCore`]: the app's session behind one `Send + Sync` handle, for the Tauri shell.
//!
//! # Locking
//!
//! `AppCore` has these locks, each held only for a few field reads or writes:
//! the job slot, `settings`, `engine`, `session`, `announced` and the image caches (`image_cache`). No lock is held while a
//! photo is read, a model loads or runs, an image is encoded, or progress is reported. The only
//! nesting is `engine` -> `settings` (reading `prefer_gpu` while the engine is created); nothing
//! takes them the other way round, so the locks cannot deadlock.
//!
//! [`AppCore::match_all`] also has a counter lock of its own, held while each `Matching` event
//! is sent so that events go out one at a time. A worker takes it only after it has released the
//! session, and no `AppCore` lock is ever held when it is taken, so a callback that calls back
//! into `AppCore` (to cancel, say) cannot deadlock either.
//!
//! A long call (detection, model loading, the saved-watermark search, matching, adding, removing or
//! clearing photos, confirming a group's area, clearing the groups, the preview, the removal run,
//! saving the watermark, removing the downloaded data) holds the job slot from start to end, so
//! only one runs at a time and another one gets [`AppError::Busy`]. It locks the session to copy
//! its inputs, works without it, and locks it again to store the results (matching stores each
//! photo's result as soon as it is ready, and so does the removal run). Because the photo list
//! changes only inside the job slot, the photos a job read are still there when it stores the
//! results. Everything else (the photo list, areas, drafts, photo states and edits, images,
//! settings, the saved-watermark list) answers while a job runs; a match result never replaces an
//! area the person saved for that photo in the meantime, and the removal run works with the photos
//! and areas it copied when it started.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use image::RgbImage;
use puw_core::mask::{self, Mask};
use puw_core::{
    Engine, EngineOptions, Error, Guess, GuessOptions, Library, MatchStatus, ModelKind, ModelPaths,
    Progress, SetLayers, WatermarkTemplate, build_set_layers, build_template, find, guess_photo,
    load, pick_representative, representative_sample,
};
use serde::Serialize;

use crate::device::{self, DeviceFix, DeviceStatus, Platform};
use crate::download::{self, model_assets};
use crate::image_cache::ImageCache;
use crate::images::{self, area_from_edit};
use crate::jobs::{Events, Job, JobSlot, LoadPurpose, ProgressEvent, with_ticker};
use crate::photos::{AddResult, PhotoId, PhotoInfo, Problem};
use crate::session::{Area, Draft, Session};
use crate::{AppError, DataDirs, OutputRule, Result, Settings, load_fraction};

mod remove;
mod saved;
mod step3;

pub use remove::{Done, Failed, OutputCheck, PREVIEW_LONG_SIDE, RemoveReport};
pub use saved::{SavedInfo, default_watermark_name};
pub use step3::{AreaStatus, GroupInfo, GroupsState, MatchSummary, PhotoState};

/// Load speed assumed for a model that was never loaded on this computer.
const LOAD_BYTES_PER_SECOND: f32 = 20_000_000.0;
/// Photos a saved watermark is tried on before it is offered.
const SUGGEST_PHOTOS: usize = 5;

/// The photos the step-2 editor can show, and the one it opens first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectOutcome {
    pub sample: Vec<PhotoId>,
    pub representative: PhotoId,
}

/// A saved watermark that seems to be on these photos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub id: String,
    pub name: String,
}

/// The app's session: settings, engine, photos, areas and cached images.
pub struct AppCore {
    dirs: DataDirs,
    jobs: JobSlot,
    settings: Mutex<Settings>,
    engine: Mutex<Option<Arc<Engine>>>,
    session: Mutex<Session>,
    images: ImageCache,
    /// Models of the current engine whose move to the processor was announced
    /// ([`ProgressEvent::FellBackToProcessor`]).
    announced: Mutex<HashSet<ModelKind>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn not_found(what: &str) -> AppError {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        what.to_string(),
    ))
    .into()
}

fn no_photo(id: PhotoId) -> AppError {
    not_found(&format!("no photo with id {id}"))
}

fn model_file(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Segmenter => "segmenter.onnx",
        ModelKind::Lama => "lama.onnx",
        ModelKind::Face => "face.onnx",
    }
}

/// `cap` evenly spaced indexes into `0..n`, or all of them when `n <= cap`.
fn spread(n: usize, cap: usize) -> Vec<usize> {
    if n <= cap {
        return (0..n).collect();
    }
    (0..cap).map(|k| (2 * k + 1) * n / (2 * cap)).collect()
}

impl AppCore {
    /// Opens the data folders (creating missing ones) and reads the settings. Loads no model.
    pub fn new(dirs: DataDirs) -> Result<AppCore> {
        dirs.ensure()?;
        let settings = Settings::load(&dirs.settings);
        Ok(AppCore {
            dirs,
            jobs: JobSlot::default(),
            settings: Mutex::new(settings),
            engine: Mutex::new(None),
            session: Mutex::new(Session::default()),
            images: ImageCache::new(),
            announced: Mutex::new(HashSet::new()),
        })
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    /// Saves new settings. The measured load times stay as they are. A change of `prefer_gpu`
    /// drops the engine; the models load again when next needed (a running job keeps its own).
    pub fn set_settings(&self, new: Settings) -> Result<()> {
        if let OutputRule::NextToOriginals { folder_name } = &new.output
            && !crate::settings::valid_folder_name(folder_name)
        {
            return Err(AppError::BadFolderName);
        }
        let gpu_changed = {
            let mut current = lock(&self.settings);
            let new = Settings {
                load_seconds: current.load_seconds.clone(),
                ..new
            };
            new.save(&self.dirs.settings)?;
            let changed = new.prefer_gpu != current.prefer_gpu;
            *current = new;
            changed
        };
        if gpu_changed {
            self.drop_engine();
        }
        Ok(())
    }

    /// Drops the engine; the models load again when next needed (a running job keeps its own).
    fn drop_engine(&self) {
        *lock(&self.engine) = None;
        lock(&self.announced).clear();
    }

    /// Where the loaded models run, for the device chip. Creates no engine.
    pub fn device_status(&self) -> DeviceStatus {
        let report = lock(&self.engine).as_ref().map(|e| e.report());
        device::device_status(
            report.as_ref(),
            lock(&self.settings).prefer_gpu,
            Platform::current(),
        )
    }

    /// Applies a fix from the "why?" dialog: the models load again when next needed, with the
    /// processor or the graphics card as chosen. [`DeviceFix::DownloadHelper`] does nothing yet
    /// (it is never offered before the helper download exists). Refused with
    /// [`AppError::Busy`] while a job runs, so the models of a running job stay where the chip
    /// says they are.
    pub fn device_fix(&self, fix: DeviceFix) -> Result<()> {
        let _job = self.jobs.start()?;
        let prefer_gpu = match fix {
            DeviceFix::TryAgain => None,
            DeviceFix::UseProcessor => Some(false),
            DeviceFix::UseGraphicsCard => Some(true),
            DeviceFix::DownloadHelper { .. } => return Ok(()),
        };
        if let Some(prefer_gpu) = prefer_gpu {
            self.set_settings(Settings {
                prefer_gpu,
                ..self.settings()
            })?;
        }
        self.drop_engine();
        Ok(())
    }

    /// The engine, created on first use with the current settings. Loads no model.
    pub fn engine(&self) -> Result<Arc<Engine>> {
        let mut engine = lock(&self.engine);
        if let Some(e) = engine.as_ref() {
            return Ok(e.clone());
        }
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let opts = EngineOptions {
            prefer_gpu: lock(&self.settings).prefer_gpu,
            gpu_helper_dir: None,
            intra_threads: Some(threads.saturating_sub(1).max(1)),
            cache_dir: Some(self.dirs.cache.clone()),
        };
        let paths = ModelPaths::from_dir(&self.dirs.models)
            .map_err(|e| AppError::MissingModels(e.to_string()))?;
        let e = Arc::new(Engine::new(&paths, opts)?);
        *engine = Some(e.clone());
        Ok(e)
    }

    /// Asks the running job, if any, to stop.
    pub fn cancel(&self) {
        self.jobs.cancel();
    }

    /// Asks the job started with `ticket` ([`crate::jobs::with_ticket`]) to stop, and no other:
    /// the running one when it is that job, else that job as soon as it starts.
    pub fn cancel_ticket(&self, ticket: u64) {
        self.jobs.cancel_ticket(ticket);
    }

    /// Deletes the downloaded models, the saved watermarks and the model cache (the settings
    /// stay) and leaves empty folders behind. The loaded models are dropped too. Fails with
    /// [`AppError::Busy`] while a job runs, and no job can start until it is done.
    pub fn remove_downloaded_data(&self) -> Result<()> {
        let _job = self.jobs.start()?;
        self.drop_engine();
        download::remove_downloaded_data(&self.dirs)?;
        self.dirs.ensure()
    }

    fn session(&self) -> MutexGuard<'_, Session> {
        lock(&self.session)
    }

    /// Adds files and folders. Results of earlier runs (in the output folder) are left out.
    /// When a photo is added, every group is cleared ([`AppCore::clear_groups`]).
    pub fn add_paths(&self, paths: &[PathBuf]) -> Result<AddResult> {
        let _job = self.jobs.start()?;
        let rule = self.settings().output;
        // Photo files are read without the lock; the job slot keeps the list from changing.
        let mut list = self.session().photos.clone();
        let result = list.add_paths(paths, &rule);
        let mut s = self.session();
        s.photos = list;
        if !result.added.is_empty() && !s.groups.is_empty() {
            s.clear_groups();
        }
        Ok(result)
    }

    /// Removes photo `id` from the list; every group is cleared ([`AppCore::clear_groups`]).
    pub fn remove_photo(&self, id: PhotoId) -> Result<()> {
        let _job = self.jobs.start()?;
        let mut s = self.session();
        s.remove(id);
        if !s.groups.is_empty() {
            s.clear_groups();
        }
        drop(s);
        self.images.remove(id);
        Ok(())
    }

    /// Forgets all photos and everything learned from them, for a new set.
    pub fn clear_photos(&self) -> Result<()> {
        let _job = self.jobs.start()?;
        self.session().clear();
        self.images.clear();
        Ok(())
    }

    pub fn photos(&self) -> Vec<PhotoInfo> {
        self.session().photos.infos()
    }

    /// Ids and files of all photos, in list order.
    fn photo_files(&self) -> (Vec<PhotoId>, Vec<PathBuf>) {
        let s = self.session();
        let ids = s.photos.infos().iter().map(|i| i.id).collect();
        (ids, s.photos.paths())
    }

    fn photo_path(&self, id: PhotoId) -> Result<PathBuf> {
        let s = self.session();
        Ok(s.photos.get(id).ok_or_else(|| no_photo(id))?.path.clone())
    }

    /// Loads the models in `kinds` that are not loaded yet, reporting [`ProgressEvent::LoadingModels`].
    pub fn ensure_models(
        &self,
        kinds: &[ModelKind],
        purpose: LoadPurpose,
        ev: Events,
    ) -> Result<()> {
        let job = self.jobs.start()?;
        self.ensure_models_in(&job, kinds, purpose, ev).map(|_| ())
    }

    /// [`AppCore::ensure_models`] inside a running job; returns the engine the models are in.
    ///
    /// Sends nothing when every model is loaded. Otherwise progress starts at 0.0, is estimated
    /// every 200 ms from the expected load times (never above 0.95) and ends at 1.0.
    fn ensure_models_in(
        &self,
        job: &Job,
        kinds: &[ModelKind],
        purpose: LoadPurpose,
        ev: Events,
    ) -> Result<Arc<Engine>> {
        let engine = self.engine()?;
        let mut missing: Vec<ModelKind> = Vec::new();
        for &k in kinds {
            if engine.backend(k).is_none() && !missing.contains(&k) {
                missing.push(k);
            }
        }
        if missing.is_empty() {
            return Ok(engine);
        }
        let expected: Vec<f32> = missing
            .iter()
            .map(|&k| self.expected_load_seconds(k))
            .collect();
        let total: f32 = expected.iter().sum();
        // The highest fraction sent so far, shared with the ticker thread; the bar never goes back.
        let last = Mutex::new(0.0f32);
        let emit = |fraction: f32| {
            let fraction = {
                let mut last = lock(&last);
                *last = last.max(fraction);
                *last
            };
            ev(ProgressEvent::LoadingModels {
                fraction,
                which: purpose,
            });
        };
        emit(0.0);
        let mut base = 0.0;
        for (&kind, &secs) in missing.iter().zip(&expected) {
            job.check()?;
            // A file deleted since the engine was made is missing data, not a photo problem.
            let file = self.dirs.models.join(model_file(kind));
            if !file.is_file() {
                self.drop_engine();
                return Err(AppError::MissingModels(file.display().to_string()));
            }
            let start = Instant::now();
            with_ticker(&|t| emit(load_fraction(base + t, total)), || {
                engine.load(kind)
            })?;
            self.record_load_seconds(kind, start.elapsed().as_secs_f32());
            base += secs;
        }
        emit(1.0);
        Ok(engine)
    }

    /// Seconds `kind` took to load last time, else an estimate from its file size.
    fn expected_load_seconds(&self, kind: ModelKind) -> f32 {
        if let Some(&s) = lock(&self.settings).load_seconds.get(&kind)
            && s > 0.0
        {
            return s;
        }
        let bytes = model_assets()
            .iter()
            .find(|a| a.name == model_file(kind))
            .map_or(0, |a| a.bytes);
        bytes as f32 / LOAD_BYTES_PER_SECOND
    }

    fn record_load_seconds(&self, kind: ModelKind, seconds: f32) {
        let mut s = lock(&self.settings);
        s.load_seconds.insert(kind, seconds);
        // Only a hint for the next progress bar: a failed save must not fail the job.
        let _ = s.save(&self.dirs.settings);
    }

    /// A saved watermark found on most of the first few photos (no model needed).
    pub fn suggest_saved(&self) -> Result<Option<Suggestion>> {
        let job = self.jobs.start()?;
        let library = Library::open(&self.dirs.watermarks)?;
        let saved = library.list()?;
        if saved.is_empty() {
            return Ok(None);
        }
        let (_, paths) = self.photo_files();
        let mut imgs: Vec<RgbImage> = Vec::new();
        for i in spread(paths.len(), SUGGEST_PHOTOS) {
            job.check()?;
            // A photo that cannot be read just does not take part.
            if let Ok(p) = load(&paths[i]) {
                imgs.push(p.rgb);
            }
        }
        let refs: Vec<&RgbImage> = imgs.iter().collect();
        let Some((id, _)) = library.suggest_until(&refs, job.flag())? else {
            return Ok(None);
        };
        Ok(saved.into_iter().find(|w| w.id == id).map(|w| Suggestion {
            id: w.id,
            name: w.name,
        }))
    }

    /// Finds the watermark on a few photos ([`representative_sample`], leaving out any that
    /// cannot be read) after loading the detection models and reading the set (its batch layers
    /// are kept). The guesses become those photos' step-2 areas ([`AppCore::start_area`]);
    /// every step-2 guess and draft from before is dropped.
    pub fn detect(&self, ev: Events) -> Result<DetectOutcome> {
        let job = self.jobs.start()?;
        let (ids, paths) = self.photo_files();
        if ids.is_empty() {
            return Err(not_found("no photos to look at"));
        }
        let engine = self.ensure_models_in(
            &job,
            &[ModelKind::Segmenter, ModelKind::Face],
            LoadPurpose::Finder,
            ev,
        )?;
        let layers = self.read_layers(&job, ev, &ids, &paths)?;

        let opts = GuessOptions::default();
        let looked = self.look_at_sample(&job, &ids, &paths, ev, |photo| {
            guess_photo(&engine, photo, &layers, &opts)
        })?;
        let (sample, guesses): (Vec<PhotoId>, Vec<Guess>) = looked.into_iter().unzip();
        let representative = sample[pick_representative(&guesses)];

        let outcome = DetectOutcome {
            sample,
            representative,
        };
        let mut s = self.session();
        s.set_guesses(
            outcome
                .sample
                .iter()
                .copied()
                .zip(guesses.into_iter().map(|g| g.mask))
                .collect(),
        );
        s.last_detect = Some(outcome.clone());
        Ok(outcome)
    }

    /// Looks for saved watermark `id` on the same photos [`AppCore::detect`] would use, without
    /// any model. The best `Found` photo comes first, else the first photo. Like `detect`, it
    /// replaces every step-2 guess and drops every draft.
    pub fn use_saved(&self, id: &str, ev: Events) -> Result<DetectOutcome> {
        let job = self.jobs.start()?;
        let saved = Library::open(&self.dirs.watermarks)?
            .list()?
            .into_iter()
            .find(|w| w.id == id)
            .ok_or_else(|| not_found("no such saved watermark"))?;
        let (ids, paths) = self.photo_files();
        if ids.is_empty() {
            return Err(not_found("no photos to look at"));
        }
        let results = self.look_at_sample(&job, &ids, &paths, ev, |p| {
            Ok(find(&saved.template, &p.rgb))
        })?;
        let sample: Vec<PhotoId> = results.iter().map(|(id, _)| *id).collect();
        let representative = results
            .iter()
            .filter(|(_, r)| r.status == MatchStatus::Found)
            .max_by(|a, b| a.1.confidence.total_cmp(&b.1.confidence))
            .map_or(sample[0], |(id, _)| *id);

        let outcome = DetectOutcome {
            sample,
            representative,
        };
        let mut s = self.session();
        s.set_guesses(results.into_iter().map(|(id, r)| (id, r.mask)).collect());
        s.last_detect = Some(outcome.clone());
        Ok(outcome)
    }

    /// Reads photos `ids` (files `paths`) for their batch layers ([`ProgressEvent::Reading`])
    /// and keeps them, built from those photos.
    fn read_layers(
        &self,
        job: &Job,
        ev: Events,
        ids: &[PhotoId],
        paths: &[PathBuf],
    ) -> Result<Arc<SetLayers>> {
        let on_reading = |fraction: f32| ev(ProgressEvent::Reading { fraction });
        let progress = Progress {
            on_fraction: &on_reading,
            cancel: job.flag(),
        };
        let layers = Arc::new(build_set_layers(paths, &progress)?);
        self.session().set_layers(layers.clone(), ids);
        Ok(layers)
    }

    /// Runs `look` on each photo of the [`representative_sample`], one at a time, reporting
    /// [`ProgressEvent::Looking`]. A photo that cannot be read is left out; when none can be
    /// read, the first one's error is returned. Never empty.
    fn look_at_sample<T>(
        &self,
        job: &Job,
        ids: &[PhotoId],
        paths: &[PathBuf],
        ev: Events,
        look: impl Fn(&puw_core::LoadedPhoto) -> puw_core::Result<T>,
    ) -> Result<Vec<(PhotoId, T)>> {
        let picks = representative_sample(ids.len());
        let total = picks.len() as u32;
        ev(ProgressEvent::Looking { done: 0, total });
        let mut found = Vec::new();
        let mut unreadable = None;
        for (done, &i) in picks.iter().enumerate() {
            job.check()?;
            match load(&paths[i]) {
                Ok(photo) => found.push((ids[i], look(&photo)?)),
                Err(e) => {
                    let e = AppError::from(e);
                    if let Some(p) = Problem::of(&e) {
                        self.session().photos.set_problem(ids[i], p);
                    }
                    unreadable = unreadable.or(Some(e));
                }
            }
            ev(ProgressEvent::Looking {
                done: done as u32 + 1,
                total,
            });
        }
        match unreadable {
            Some(e) if found.is_empty() => Err(e),
            _ => Ok(found),
        }
    }

    /// Size of the image the area of photo `id` is drawn on.
    pub fn edit_size(&self, id: PhotoId) -> Result<(u32, u32)> {
        let s = self.session();
        let info = &s.photos.get(id).ok_or_else(|| no_photo(id))?.info;
        Ok(images::edit_size(info.width, info.height))
    }

    /// The photo's area at edit size, one byte per pixel (0 or 255), row by row: its own area,
    /// else where its group's watermark was found, else empty.
    pub fn area(&self, id: PhotoId) -> Result<Vec<u8>> {
        self.area_at_edit_size(id, Session::area)
    }

    /// The area step 2 starts from, at edit size like [`AppCore::area`]: the step-2 guess when
    /// the photo has one and is in no group, else its area.
    pub fn start_area(&self, id: PhotoId) -> Result<Vec<u8>> {
        self.area_at_edit_size(id, Session::start_area)
    }

    fn area_at_edit_size(
        &self,
        id: PhotoId,
        which: fn(&Session, PhotoId) -> Option<Arc<Area>>,
    ) -> Result<Vec<u8>> {
        let (area, (ew, eh)) = {
            let s = self.session();
            let info = &s.photos.get(id).ok_or_else(|| no_photo(id))?.info;
            (which(&s, id), images::edit_size(info.width, info.height))
        };
        Ok(match area {
            Some(a) => a.resized(ew, eh).into_raw(),
            None => vec![0; ew as usize * eh as usize],
        })
    }

    /// Keeps the area the editor shows for photo `id` (edit size, non-zero = area).
    pub fn put_draft(&self, id: PhotoId, w: u32, h: u32, bytes: Vec<u8>) -> Result<()> {
        let mut s = self.session();
        let info = &s.photos.get(id).ok_or_else(|| no_photo(id))?.info;
        let edit = images::edit_size(info.width, info.height);
        if (w, h) != edit || bytes.len() != w as usize * h as usize {
            return Err(Error::InvalidInput(format!(
                "area of {} values at {w}x{h}, expected {}x{}",
                bytes.len(),
                edit.0,
                edit.1
            ))
            .into());
        }
        s.data.entry(id).or_default().draft = Some(Draft { w, h, bytes });
        Ok(())
    }

    /// Forgets the draft of photo `id` (the editor was left without saving it).
    pub fn drop_draft(&self, id: PhotoId) {
        if let Some(d) = self.session().data.get_mut(&id) {
            d.draft = None;
        }
    }

    /// Makes the draft of photo `id` (or, without one, its step-2 area, see
    /// [`AppCore::start_area`]) the current group's watermark area and learns what it looks
    /// like; with no group yet, this makes group 1. Every photo whose result came from the
    /// current group waits for matching again (areas saved for single photos stay, and so do
    /// earlier groups). Fails with [`AppError::NothingToRemove`] when the area is empty.
    /// Confirming, on a photo the current group found and that has no area of its own, the
    /// very area it was found with changes nothing (step 3 keeps its results).
    pub fn confirm_set_area(&self, id: PhotoId) -> Result<()> {
        self.confirm_area(id, false)
    }

    /// Like [`AppCore::confirm_set_area`], but makes the next group (group 1 when there is
    /// none), which becomes the current one. Every photo in no group is reset first; the
    /// photos of earlier groups stay as they are. Fails with [`AppError::NotReady`] until the
    /// current group's matching has finished and no unsure photo holds it up.
    pub fn confirm_new_group(&self, id: PhotoId) -> Result<()> {
        self.confirm_area(id, true)
    }

    /// `Ok` when the next group can start: there is no group, or the current group's matching
    /// has finished and [`AppCore::removal_blockers`] is empty. Else [`AppError::NotReady`].
    fn ready_for_next_group(&self) -> Result<()> {
        if self.session().ready_for_next_group() {
            Ok(())
        } else {
            Err(AppError::NotReady)
        }
    }

    fn confirm_area(&self, id: PhotoId, new_group: bool) -> Result<()> {
        let _job = self.jobs.start()?;
        if new_group {
            self.ready_for_next_group()?;
        }
        let (path, (w, h), draft, start) = {
            let s = self.session();
            let entry = s.photos.get(id).ok_or_else(|| no_photo(id))?;
            let d = s.data.get(&id);
            (
                entry.path.clone(),
                (entry.info.width, entry.info.height),
                d.and_then(|d| d.draft.clone()),
                s.start_area(id),
            )
        };
        let area = match (draft, start) {
            (Some(d), _) => area_from_edit(&d.bytes, d.w, d.h, w, h)?,
            (None, Some(a)) => a.to_full(),
            (None, None) => Mask::new(w, h),
        };
        if mask::is_empty(&area) {
            return Err(AppError::NothingToRemove);
        }
        // The same area confirmed again (step 3, Back, "Looks right" unchanged): nothing to
        // learn, and every match, tick and look in step 3 stays as it is.
        if !new_group && area.dimensions() == (w, h) {
            let mut s = self.session();
            if s.confirms_nothing_new(id, &Area::from_mask(&area)) {
                if let Some(d) = s.data.get_mut(&id) {
                    d.draft = None;
                }
                return Ok(());
            }
        }
        let photo = load(&path)?;
        let area = if area.dimensions() == photo.rgb.dimensions() {
            area
        } else {
            mask::binarize(&mask::resize_any(
                &area,
                photo.rgb.width(),
                photo.rgb.height(),
            ))
        };
        let template = build_template(&photo.rgb, &area);
        if template.pieces.is_empty() {
            return Err(AppError::NothingToRemove);
        }
        let mut s = self.session();
        if new_group {
            // Ticks and opens do not need the job slot; check again with the results stored.
            if !s.ready_for_next_group() {
                return Err(AppError::NotReady);
            }
            s.confirm_new(id, template, &area);
        } else {
            s.confirm_current(id, template, &area);
        }
        Ok(())
    }

    /// What the current group's watermark looks like, once there is a group.
    pub fn confirmed_template(&self) -> Option<Arc<WatermarkTemplate>> {
        self.session().current().map(|g| g.template.clone())
    }

    /// Small JPEG of photo `id` (long side [`images::THUMB_LONG_SIDE`]), kept in memory. A
    /// photo that cannot be read gets its [`crate::photos::Problem`].
    pub fn thumb_image(&self, id: PhotoId) -> Result<Vec<u8>> {
        let path = self.photo_path(id)?;
        self.noting_problem(id, self.images.thumb(id, &path))
    }

    /// JPEG of photo `id` at edit size, the image the area is drawn on, kept in memory. A
    /// photo that cannot be read gets its [`crate::photos::Problem`].
    pub fn edit_image(&self, id: PhotoId) -> Result<Vec<u8>> {
        let path = self.photo_path(id)?;
        self.noting_problem(id, self.images.edit(id, &path))
    }

    /// `result`, after noting the photo's problem when it is a failed read of photo `id`.
    fn noting_problem<T>(&self, id: PhotoId, result: Result<T>) -> Result<T> {
        if let Err(e) = &result
            && let Some(p) = Problem::of(e)
        {
            self.session().photos.set_problem(id, p);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spread_is_even() {
        assert_eq!(spread(3, 5), vec![0, 1, 2]);
        assert_eq!(spread(10, 5), vec![1, 3, 5, 7, 9]);
    }

    #[test]
    fn expected_load_time_from_size_until_measured() {
        let t = tempfile::tempdir().unwrap();
        let core = AppCore::new(DataDirs::at(t.path())).unwrap();
        let seg = core.expected_load_seconds(ModelKind::Segmenter);
        assert!((seg - 480_487_402.0 / 20e6).abs() < 0.01, "{seg}");
        core.record_load_seconds(ModelKind::Segmenter, 3.5);
        assert_eq!(core.expected_load_seconds(ModelKind::Segmenter), 3.5);
        assert_eq!(
            Settings::load(&core.dirs.settings).load_seconds[&ModelKind::Segmenter],
            3.5
        );
    }

    #[test]
    fn bad_folder_names_are_refused() {
        let t = tempfile::tempdir().unwrap();
        let core = AppCore::new(DataDirs::at(t.path())).unwrap();
        for name in ["", "..", "a/b", "x:y"] {
            let s = Settings {
                output: OutputRule::NextToOriginals {
                    folder_name: name.into(),
                },
                ..Settings::default()
            };
            assert!(matches!(core.set_settings(s), Err(AppError::BadFolderName)));
        }
        assert_eq!(core.settings().output, OutputRule::default());
    }

    #[test]
    fn settings_keep_measured_load_times() {
        let t = tempfile::tempdir().unwrap();
        let core = AppCore::new(DataDirs::at(t.path())).unwrap();
        core.record_load_seconds(ModelKind::Face, 0.5);
        core.set_settings(Settings {
            prefer_gpu: false,
            ..Settings::default()
        })
        .unwrap();
        let s = core.settings();
        assert!(!s.prefer_gpu);
        assert_eq!(s.load_seconds[&ModelKind::Face], 0.5);
        assert_eq!(Settings::load(&core.dirs.settings), s);
    }

    #[test]
    fn device_fix_drops_the_engine_and_sets_the_choice() {
        let t = tempfile::tempdir().unwrap();
        let core = AppCore::new(DataDirs::at(t.path())).unwrap();
        assert_eq!(core.device_status().state, crate::DeviceState::NotStarted);
        let models = crate::download::model_assets();
        for a in &models {
            std::fs::write(core.dirs.models.join(a.name), b"x").unwrap();
        }
        let first = core.engine().unwrap();
        core.device_fix(DeviceFix::TryAgain).unwrap();
        assert!(!Arc::ptr_eq(&first, &core.engine().unwrap()));
        assert!(core.settings().prefer_gpu);

        core.device_fix(DeviceFix::UseProcessor).unwrap();
        assert!(!core.settings().prefer_gpu);
        assert!(!Settings::load(&core.dirs.settings).prefer_gpu);
        let second = core.engine().unwrap();
        core.device_fix(DeviceFix::UseProcessor).unwrap();
        assert!(!Arc::ptr_eq(&second, &core.engine().unwrap()));

        core.device_fix(DeviceFix::UseGraphicsCard).unwrap();
        assert!(core.settings().prefer_gpu);
        assert_eq!(core.device_status().state, crate::DeviceState::NotStarted);

        // Not while a job runs: the engine and the choice stay.
        let third = core.engine().unwrap();
        {
            let _job = core.jobs.start().unwrap();
            assert!(matches!(
                core.device_fix(DeviceFix::UseProcessor),
                Err(AppError::Busy)
            ));
        }
        assert!(core.settings().prefer_gpu);
        assert!(Arc::ptr_eq(&third, &core.engine().unwrap()));
    }

    #[test]
    fn remove_downloaded_data_is_refused_during_a_job() {
        let t = tempfile::tempdir().unwrap();
        let core = AppCore::new(DataDirs::at(t.path())).unwrap();
        let model = core.dirs.models.join("lama.onnx");
        std::fs::write(&model, b"x").unwrap();
        {
            let _job = core.jobs.start().unwrap();
            assert!(matches!(core.remove_downloaded_data(), Err(AppError::Busy)));
            assert!(model.exists());
        }
        core.set_settings(Settings::default()).unwrap();
        core.remove_downloaded_data().unwrap();
        assert!(!model.exists());
        assert!(core.dirs.models.is_dir() && core.dirs.watermarks.is_dir());
        assert!(core.dirs.settings.is_file());
    }
}
