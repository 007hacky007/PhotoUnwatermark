//! [`AppCore`]: the app's session behind one `Send + Sync` handle, for the Tauri shell.
//!
//! # Locking
//!
//! `AppCore` has these locks, each held only for a few field reads or writes:
//! the job slot, `settings`, `engine`, `session` and the two image caches (`image_cache`). No lock is held while a
//! photo is read, a model loads or runs, an image is encoded, or progress is reported. The only
//! nesting is `engine` -> `settings` (reading `prefer_gpu` while the engine is created); nothing
//! takes them the other way round, so the locks cannot deadlock.
//!
//! [`AppCore::match_all`] also has a counter lock of its own, held while each `Matching` event
//! is sent so that events go out one at a time. A worker takes it only after it has released the
//! session, and no `AppCore` lock is ever held when it is taken, so a callback that calls back
//! into `AppCore` (to cancel, say) cannot deadlock either.
//!
//! A long call (detection, model loading, the saved-watermark search, matching, adding, removing
//! or clearing photos, confirming the set area) holds the job slot from start to end, so only one
//! runs at a time and another one gets [`AppError::Busy`]. It locks the session to copy its
//! inputs, works without it, and locks it again to store the results (matching stores each
//! photo's result as soon as it is ready). Because the photo list changes only inside the job
//! slot, the photos a job read are still there when it stores the results. Everything else (the
//! photo list, areas, drafts, photo states and edits, images, settings) answers while a job runs;
//! a match result never replaces an area the person saved for that photo in the meantime.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use image::RgbImage;
use puw_core::mask::{self, Mask};
use puw_core::{
    Engine, EngineOptions, Error, Guess, GuessOptions, Library, MatchStatus, ModelKind, ModelPaths,
    Progress, SetLayers, WatermarkTemplate, build_set_layers, build_template, find, guess_photo,
    load, match_photo, pick_representative, representative_sample,
};
use rayon::prelude::*;
use serde::Serialize;

use crate::download::model_assets;
use crate::image_cache::ImageCache;
use crate::images::{self, GRID_LONG_SIDE, area_from_edit, jpeg, overlay, thumbnail};
use crate::jobs::{Events, Job, JobSlot, LoadPurpose, ProgressEvent, with_ticker};
use crate::photos::{AddResult, PhotoId, PhotoInfo};
use crate::session::{Area, Draft, Matched, Session, Unmatched};
use crate::{AppError, DataDirs, OutputRule, Result, Settings, load_fraction};

/// Load speed assumed for a model that was never loaded on this computer.
const LOAD_BYTES_PER_SECOND: f32 = 20_000_000.0;
/// Photos a saved watermark is tried on before it is offered.
const SUGGEST_PHOTOS: usize = 5;
const GRID_QUALITY: u8 = 85;

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

/// How finding the set's watermark went, over all photos. A photo with an area of its own
/// counts as found, or as not found when that area is empty; a photo not looked for yet
/// counts only in `total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchSummary {
    pub found: u32,
    pub unsure: u32,
    pub not_found: u32,
    pub total: u32,
}

/// What step 3 shows about a photo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AreaStatus {
    Found,
    Unsure,
    NotFound,
    /// The person saved an area for this photo alone.
    Edited,
    /// Not looked for yet (no set area, or added after matching).
    Pending,
}

/// One photo as step 3 shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoState {
    pub id: PhotoId,
    pub status: AreaStatus,
    /// Ticked for removal.
    pub included: bool,
    /// Opened in the editor since its result came.
    pub reviewed: bool,
    /// Its current area is not empty.
    pub has_area: bool,
}

/// The app's session: settings, engine, photos, areas and cached images.
pub struct AppCore {
    dirs: DataDirs,
    jobs: JobSlot,
    settings: Mutex<Settings>,
    engine: Mutex<Option<Arc<Engine>>>,
    session: Mutex<Session>,
    images: ImageCache,
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
        })
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    /// Saves new settings. The measured load times stay as they are. A change of `prefer_gpu`
    /// drops the engine; the models load again when next needed (a running job keeps its own).
    pub fn set_settings(&self, new: Settings) -> Result<()> {
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
            *lock(&self.engine) = None;
        }
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
        let e = Arc::new(Engine::new(
            &ModelPaths::from_dir(&self.dirs.models)?,
            opts,
        )?);
        *engine = Some(e.clone());
        Ok(e)
    }

    /// Asks the running job, if any, to stop.
    pub fn cancel(&self) {
        self.jobs.cancel();
    }

    fn session(&self) -> MutexGuard<'_, Session> {
        lock(&self.session)
    }

    /// Adds files and folders. Results of earlier runs (in the output folder) are left out.
    pub fn add_paths(&self, paths: &[PathBuf]) -> Result<AddResult> {
        let _job = self.jobs.start()?;
        let output_name = match &self.settings().output {
            OutputRule::NextToOriginals { folder_name } => folder_name.clone(),
            OutputRule::Folder { path } => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        // Photo files are read without the lock; the job slot keeps the list from changing.
        let mut list = self.session().photos.clone();
        let result = list.add_paths(paths, &output_name);
        self.session().photos = list;
        Ok(result)
    }

    pub fn remove_photo(&self, id: PhotoId) -> Result<()> {
        let _job = self.jobs.start()?;
        self.session().remove(id);
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
        let Some((id, _)) = library.suggest(&refs)? else {
            return Ok(None);
        };
        Ok(saved.into_iter().find(|w| w.id == id).map(|w| Suggestion {
            id: w.id,
            name: w.name,
        }))
    }

    /// Finds the watermark on a few photos ([`representative_sample`], leaving out any that
    /// cannot be read) after loading the detection models and reading the set (its batch layers
    /// are kept). The guesses become those photos' areas; every step-2 guess and draft from
    /// before is dropped.
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
        let layers = self.read_layers(&job, ev)?;

        let opts = GuessOptions::default();
        let looked = self.look_at_sample(&job, &ids, &paths, ev, |photo| {
            guess_photo(&engine, photo, &layers, &opts)
        })?;
        let (sample, guesses): (Vec<PhotoId>, Vec<Guess>) = looked.into_iter().unzip();
        let representative = sample[pick_representative(&guesses)];

        self.session().set_guesses(
            sample
                .iter()
                .copied()
                .zip(guesses.into_iter().map(|g| g.mask))
                .collect(),
        );
        Ok(DetectOutcome {
            sample,
            representative,
        })
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

        self.session()
            .set_guesses(results.into_iter().map(|(id, r)| (id, r.mask)).collect());
        Ok(DetectOutcome {
            sample,
            representative,
        })
    }

    /// Reads the whole set for its batch layers ([`ProgressEvent::Reading`]) and keeps them.
    fn read_layers(&self, job: &Job, ev: Events) -> Result<Arc<SetLayers>> {
        let (ids, paths) = self.photo_files();
        let on_reading = |fraction: f32| ev(ProgressEvent::Reading { fraction });
        let progress = Progress {
            on_fraction: &on_reading,
            cancel: job.flag(),
        };
        let layers = Arc::new(build_set_layers(&paths, &progress)?);
        self.session().set_layers(layers.clone(), &ids);
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
                Err(e) => unreadable = unreadable.or(Some(e)),
            }
            ev(ProgressEvent::Looking {
                done: done as u32 + 1,
                total,
            });
        }
        match unreadable {
            Some(e) if found.is_empty() => Err(e.into()),
            _ => Ok(found),
        }
    }

    /// Size of the image the area of photo `id` is drawn on.
    pub fn edit_size(&self, id: PhotoId) -> Result<(u32, u32)> {
        let s = self.session();
        let info = &s.photos.get(id).ok_or_else(|| no_photo(id))?.info;
        Ok(images::edit_size(info.width, info.height))
    }

    /// The photo's current area at edit size, one byte per pixel (0 or 255), row by row: its
    /// own area, else where the set's watermark was found, else the step-2 guess, else empty.
    pub fn area(&self, id: PhotoId) -> Result<Vec<u8>> {
        let (area, (ew, eh)) = {
            let s = self.session();
            let info = &s.photos.get(id).ok_or_else(|| no_photo(id))?.info;
            (s.area(id), images::edit_size(info.width, info.height))
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

    /// Makes the draft of photo `id` (or, without one, its current area) the set's watermark
    /// area and learns what it looks like. Fails with [`AppError::NothingToRemove`] when the
    /// area is empty.
    pub fn confirm_set_area(&self, id: PhotoId) -> Result<()> {
        let _job = self.jobs.start()?;
        let (path, (w, h), draft, current) = {
            let s = self.session();
            let entry = s.photos.get(id).ok_or_else(|| no_photo(id))?;
            let d = s.data.get(&id);
            (
                entry.path.clone(),
                (entry.info.width, entry.info.height),
                d.and_then(|d| d.draft.clone()),
                s.area(id),
            )
        };
        let area = match (draft, current) {
            (Some(d), _) => area_from_edit(&d.bytes, d.w, d.h, w, h)?,
            (None, Some(a)) => a.to_full(),
            (None, None) => Mask::new(w, h),
        };
        if mask::is_empty(&area) {
            return Err(AppError::NothingToRemove);
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
        self.session().confirm(id, template, &area);
        Ok(())
    }

    /// What the confirmed set area looks like, once there is one.
    pub fn confirmed_template(&self) -> Option<Arc<WatermarkTemplate>> {
        self.session().template.clone()
    }

    /// Finds the set's watermark on every photo that has neither a result nor an area of its
    /// own, reporting [`ProgressEvent::Matching`] (starting with `done: 0`). The set is read
    /// first ([`ProgressEvent::Reading`]) when it has no batch layers yet (a saved watermark
    /// was used) or photos of a size without a layer were added since.
    ///
    /// Photos are matched in parallel on at most half the cores; each worker reads, matches
    /// and drops one photo before the next. Every result is stored as soon as it is ready, so a
    /// cancelled run keeps the photos it finished. A photo that cannot be read counts as not
    /// found. Fails with [`AppError::NothingToRemove`] before a set area is confirmed.
    pub fn match_all(&self, ev: Events) -> Result<MatchSummary> {
        let job = self.jobs.start()?;
        let (template, layers, todo) = {
            let s = self.session();
            let template = s.template.clone().ok_or(AppError::NothingToRemove)?;
            let layers = if s.layers_stale() {
                None
            } else {
                s.layers.clone()
            };
            (template, layers, s.unmatched())
        };
        let layers = match layers {
            Some(l) => l,
            None if todo.is_empty() => Arc::new(SetLayers::none()),
            None => self.read_layers(&job, ev)?,
        };
        self.match_photos(&job, &template, &layers, &todo, ev)?;
        Ok(self.summary())
    }

    fn match_photos(
        &self,
        job: &Job,
        template: &WatermarkTemplate,
        layers: &SetLayers,
        todo: &[Unmatched],
        ev: Events,
    ) -> Result<()> {
        let total = todo.len() as u32;
        // Held while the event is sent, so events go out one at a time with `done` growing.
        let done = Mutex::new(0u32);
        ev(ProgressEvent::Matching { done: 0, total });
        let one = |(id, path, (w, h)): &Unmatched| -> Result<()> {
            job.check()?;
            let matched = match load(path) {
                Ok(photo) => Matched::new(&match_photo(template, &photo.rgb, layers)),
                // Nothing can be removed from a photo that cannot be read.
                Err(_) => Matched {
                    status: MatchStatus::NotFound,
                    area: Arc::new(Area::empty(*w, *h)),
                },
            };
            self.session().set_match(*id, matched);
            let mut done = lock(&done);
            *done += 1;
            ev(ProgressEvent::Matching { done: *done, total });
            Ok(())
        };
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()) / 2;
        match rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .build()
        {
            Ok(pool) => pool.install(|| todo.par_iter().try_for_each(one)),
            Err(_) => todo.iter().try_for_each(&one),
        }
    }

    /// See [`MatchSummary`].
    fn summary(&self) -> MatchSummary {
        let states = self.photo_states();
        let mut sum = MatchSummary {
            found: 0,
            unsure: 0,
            not_found: 0,
            total: states.len() as u32,
        };
        for st in &states {
            match st.status {
                AreaStatus::Found => sum.found += 1,
                AreaStatus::Edited if st.has_area => sum.found += 1,
                AreaStatus::Unsure => sum.unsure += 1,
                AreaStatus::NotFound | AreaStatus::Edited => sum.not_found += 1,
                AreaStatus::Pending => {}
            }
        }
        sum
    }

    /// Every photo's step-3 state, in list order.
    pub fn photo_states(&self) -> Vec<PhotoState> {
        self.session().states()
    }

    /// Notes that photo `id` was opened in the editor, which settles an unsure result.
    pub fn open_photo(&self, id: PhotoId) -> Result<()> {
        let mut s = self.session();
        s.photos.get(id).ok_or_else(|| no_photo(id))?;
        s.data.entry(id).or_default().reviewed = true;
        Ok(())
    }

    /// Makes the draft of photo `id` (or, without one, its current area) the area of this
    /// photo alone: status [`AreaStatus::Edited`], reviewed, and ticked when the area is not
    /// empty. The set's watermark stays as it is.
    pub fn save_photo_area(&self, id: PhotoId) -> Result<()> {
        let ((w, h), draft, current) = {
            let mut s = self.session();
            let info = &s.photos.get(id).ok_or_else(|| no_photo(id))?.info;
            let size = (info.width, info.height);
            let draft = s.data.get_mut(&id).and_then(|d| d.draft.take());
            (size, draft, s.area(id))
        };
        // The full-size area is made without the lock.
        let area = match (draft, current) {
            (Some(d), _) => Arc::new(Area::from_mask(&area_from_edit(&d.bytes, d.w, d.h, w, h)?)),
            (None, Some(a)) => a,
            (None, None) => Arc::new(Area::empty(w, h)),
        };
        let mut s = self.session();
        s.photos.get(id).ok_or_else(|| no_photo(id))?;
        s.save_own(id, area);
        Ok(())
    }

    /// Ticks or unticks photo `id` for removal. A photo whose area is empty cannot be ticked
    /// ([`AppError::NothingToRemove`]).
    pub fn set_included(&self, id: PhotoId, included: bool) -> Result<()> {
        let mut s = self.session();
        s.photos.get(id).ok_or_else(|| no_photo(id))?;
        if included && !s.has_area(id) {
            return Err(AppError::NothingToRemove);
        }
        s.data.entry(id).or_default().included = included;
        Ok(())
    }

    /// The photos that keep removal from starting, in list order: ticked unsure photos nobody
    /// opened, and photos the watermark was not looked for on yet.
    pub fn removal_blockers(&self) -> Vec<PhotoId> {
        self.photo_states()
            .into_iter()
            .filter(|st| {
                st.status == AreaStatus::Pending
                    || (st.status == AreaStatus::Unsure && st.included && !st.reviewed)
            })
            .map(|st| st.id)
            .collect()
    }

    /// The ticked photos with an area, in list order.
    pub fn to_remove(&self) -> Vec<PhotoId> {
        self.photo_states()
            .into_iter()
            .filter(|st| st.included && st.has_area)
            .map(|st| st.id)
            .collect()
    }

    /// JPEG of photo `id` for the step-3 grid (long side [`GRID_LONG_SIDE`]) with its current
    /// area tinted. Made on every call: the area may have changed.
    pub fn grid_image(&self, id: PhotoId) -> Result<Vec<u8>> {
        let (path, area) = {
            let s = self.session();
            let path = s.photos.get(id).ok_or_else(|| no_photo(id))?.path.clone();
            (path, s.area(id))
        };
        let thumb = thumbnail(&path, GRID_LONG_SIDE)?;
        let img = match area {
            Some(a) if !a.is_empty() => overlay(&thumb, &a.resized(thumb.width(), thumb.height())),
            _ => thumb,
        };
        Ok(jpeg(&img, GRID_QUALITY))
    }

    /// Small JPEG of photo `id` (long side [`images::THUMB_LONG_SIDE`]), kept in memory.
    pub fn thumb_image(&self, id: PhotoId) -> Result<Vec<u8>> {
        self.images.thumb(id, &self.photo_path(id)?)
    }

    /// JPEG of photo `id` at edit size, the image the area is drawn on, kept in memory.
    pub fn edit_image(&self, id: PhotoId) -> Result<Vec<u8>> {
        self.images.edit(id, &self.photo_path(id)?)
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
}
