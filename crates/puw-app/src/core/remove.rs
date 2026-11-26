//! Preview on one photo, step 4 (removing the watermark from every ticked photo) and the
//! before/after images. The locking rules of [`AppCore`] (see `core.rs`) apply here too.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use puw_core::engine::{Backend, CpuReason, DeviceReport};
use puw_core::inpaint::inpaint;
use puw_core::mask::{self, Mask};
use puw_core::{Engine, Error, ModelKind, Progress, load, remove_one};
use serde::Serialize;

use super::{AppCore, AreaStatus, lock, no_photo, not_found};
use crate::images::{self, jpeg};
use crate::jobs::{Events, LoadPurpose, ProgressEvent};
use crate::output::{output_dirs, remove_stale_partials};
use crate::photos::{PhotoId, Problem};
use crate::session::{Area, Made};
use crate::{AppError, Eta, OutputRule, Result, user_message};

/// Long side, in pixels, that a preview is worked at.
pub const PREVIEW_LONG_SIDE: u32 = 1024;
const IMAGE_QUALITY: u8 = 90;

/// Whether the results can be saved where they should go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputCheck {
    pub ok: bool,
    /// The photos' folders whose output folder cannot be created or written.
    pub unwritable: Vec<String>,
}

/// A photo the watermark was removed from, and the file it was saved as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    pub id: PhotoId,
    pub output: String,
}

/// A photo the watermark could not be removed from, with a sentence saying why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failed {
    pub id: PhotoId,
    pub name: String,
    pub reason: String,
}

/// How a removal run went. `cancelled` runs keep the photos finished before the stop.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveReport {
    pub done: Vec<Done>,
    pub failed: Vec<Failed>,
    /// The photos with no group when the run started, in list order. They are not touched.
    pub left_out: Vec<LeftOut>,
    pub cancelled: bool,
}

/// A photo that belongs to no group, so the run leaves it alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftOut {
    pub id: PhotoId,
    pub name: String,
}

/// One photo of a run, as it was when the run started.
struct Todo {
    id: PhotoId,
    path: PathBuf,
    name: String,
    area: Arc<Area>,
}

/// The models that moved from the graphics card to the processor because a run failed, with
/// the error text.
fn fallbacks(report: &DeviceReport) -> Vec<(ModelKind, String)> {
    [
        (ModelKind::Segmenter, &report.segmenter),
        (ModelKind::Lama, &report.lama),
        (ModelKind::Face, &report.face),
    ]
    .into_iter()
    .filter_map(|(kind, backend)| match backend {
        Some(Backend::Cpu {
            reason: CpuReason::RunFailed(text),
        }) => Some((kind, text.clone())),
        _ => None,
    })
    .collect()
}

fn is_cancelled(e: &AppError) -> bool {
    matches!(e, AppError::Core(Error::Cancelled))
}

impl AppCore {
    /// Sends [`ProgressEvent::FellBackToProcessor`] (with the first error text) when a model
    /// of `engine` moved to the processor and that was not said yet. Each model is announced
    /// once per engine, whichever job notices it first.
    fn announce_fallback(&self, engine: &Engine, ev: Events) {
        let fresh: Vec<String> = {
            let mut announced = lock(&self.announced);
            fallbacks(&engine.report())
                .into_iter()
                .filter(|(kind, _)| announced.insert(*kind))
                .map(|(_, text)| text)
                .collect()
        };
        if let Some(reason) = fresh.into_iter().next() {
            ev(ProgressEvent::FellBackToProcessor { reason });
        }
    }

    /// Tries the removal on photo `id` with its draft (or, without one, its current area):
    /// loads the remover if needed ([`ProgressEvent::LoadingModels`] with
    /// [`LoadPurpose::Remover`]), then works on the edit image at [`PREVIEW_LONG_SIDE`]
    /// ([`ProgressEvent::Preview`]). The result is kept at edit size for
    /// [`AppCore::preview_image`]. Fails with [`AppError::NothingToRemove`] when the area is
    /// empty.
    pub fn preview(&self, id: PhotoId, ev: Events) -> Result<()> {
        let job = self.jobs.start()?;
        let (path, (w, h), draft, current) = {
            let s = self.session();
            let entry = s.photos.get(id).ok_or_else(|| no_photo(id))?;
            let draft = s.data.get(&id).and_then(|d| d.draft.clone());
            (
                entry.path.clone(),
                (entry.info.width, entry.info.height),
                draft,
                s.area(id),
            )
        };
        let (ew, eh) = images::edit_size(w, h);
        let area = match (draft, current) {
            (Some(d), _) => Mask::from_raw(d.w, d.h, d.bytes)
                .map(|m| mask::binarize(&m))
                .unwrap_or_else(|| Mask::new(ew, eh)),
            (None, Some(a)) => a.resized(ew, eh),
            (None, None) => Mask::new(ew, eh),
        };
        if mask::is_empty(&area) {
            return Err(AppError::NothingToRemove);
        }
        let engine = self.ensure_models_in(&job, &[ModelKind::Lama], LoadPurpose::Remover, ev)?;
        let edit = images::edit_image(&load(&path)?);
        let area = if area.dimensions() == edit.dimensions() {
            area
        } else {
            mask::binarize(&mask::resize_any(&area, edit.width(), edit.height()))
        };
        let on_fraction = |fraction: f32| ev(ProgressEvent::Preview { fraction });
        let progress = Progress {
            on_fraction: &on_fraction,
            cancel: job.flag(),
        };
        let filled = inpaint(&engine, &edit, &area, Some(PREVIEW_LONG_SIDE), progress);
        self.announce_fallback(&engine, ev);
        let filled = filled?;
        let bytes = Arc::new(jpeg(&filled, IMAGE_QUALITY));
        self.session().data.entry(id).or_default().preview = Some(bytes);
        Ok(())
    }

    /// The last preview of photo `id` (JPEG at edit size).
    pub fn preview_image(&self, id: PhotoId) -> Result<Vec<u8>> {
        let s = self.session();
        s.photos.get(id).ok_or_else(|| no_photo(id))?;
        s.data
            .get(&id)
            .and_then(|d| d.preview.as_ref())
            .map(|b| b.to_vec())
            .ok_or_else(|| not_found("no preview of this photo"))
    }

    /// Uses `folder` for the results of this set instead of the usual output folder, or the
    /// usual one again with `None`. A new set starts without it.
    pub fn set_output_override(&self, folder: Option<PathBuf>) {
        self.session().output_override = folder;
    }

    fn output_rule(&self) -> OutputRule {
        let chosen = self.session().output_override.clone();
        match chosen {
            Some(path) => OutputRule::Folder { path },
            None => self.settings().output,
        }
    }

    /// Whether the results of the ticked photos can be saved: each output folder is created
    /// when missing and written to once.
    pub fn check_output(&self) -> Result<OutputCheck> {
        let paths: Vec<PathBuf> = {
            let s = self.session();
            s.states()
                .iter()
                .filter(|st| st.is_to_remove())
                .filter_map(|st| s.photos.get(st.id).map(|e| e.path.clone()))
                .collect()
        };
        Ok(match output_dirs(&paths, &self.output_rule()) {
            Ok(_) => OutputCheck {
                ok: true,
                unwritable: Vec::new(),
            },
            Err(bad) => OutputCheck {
                ok: false,
                unwritable: bad
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
            },
        })
    }

    /// Removes the watermark from every member of every group (each with its own area), one
    /// photo at a time, in list order, and saves the results in the output folders (the originals are never written).
    ///
    /// The photos and their areas are taken when the run starts; changes made in step 3 while
    /// it runs count from the next run on. A photo an earlier run finished with the same area,
    /// the same output folder rule and the same detail setting (and whose result is still
    /// there) is not done again: it is listed in
    /// [`RemoveReport::done`] with that result, so starting again after a stop or a failure
    /// only works on the unfinished photos. Refuses with [`AppError::NotReady`] while
    /// [`AppCore::removal_blockers`] is not empty, with [`AppError::NothingToRemove`] when no
    /// photo is ticked, and with [`AppError::NoWritableOutput`] when an output folder cannot be
    /// written, in each case before any photo is read. Then deletes the half-written files
    /// other processes left in the output folders, loads the remover if needed and works
    /// through the photos ([`ProgressEvent::Removing`]: one event when a photo starts, its
    /// progress, and one when it is finished).
    ///
    /// [`RemoveReport::left_out`] lists the photos with no group (they are not touched), and
    /// [`RemoveReport::failed`] starts with the photos already known to be unreadable.
    ///
    /// A photo that fails (moved, deleted, damaged, or the remover failed on it) is listed in
    /// [`RemoveReport::failed`] and the run goes on. Cancelling discards the photo being worked
    /// on and keeps the finished ones (`cancelled: true`). When a model moves from the graphics
    /// card to the processor (in this run, or unannounced before it),
    /// [`ProgressEvent::FellBackToProcessor`] is sent once, with the error text.
    pub fn remove(&self, ev: Events) -> Result<RemoveReport> {
        let job = self.jobs.start()?;
        let rule = self.output_rule();
        let detail = self.settings().detail;
        let (blocked, todo, finished, left_out, unreadable) = {
            let s = self.session();
            let states = s.states();
            let mut finished = Vec::new();
            let todo: Vec<Todo> = states
                .iter()
                .filter(|st| st.is_to_remove())
                .filter_map(|st| {
                    let e = s.photos.get(st.id)?;
                    let area = s.area(st.id)?;
                    if let Some(d) = s.data.get(&st.id)
                        && let (Some(out), Some(made)) = (&d.result, &d.result_made)
                        && made.same(&area, &rule, detail)
                        && out.is_file()
                    {
                        finished.push(Done {
                            id: st.id,
                            output: out.to_string_lossy().into_owned(),
                        });
                        return None;
                    }
                    Some(Todo {
                        id: st.id,
                        path: e.path.clone(),
                        name: e.info.name.clone(),
                        area,
                    })
                })
                .collect();
            let open = s.matching_open();
            let named = |id: PhotoId| {
                s.photos
                    .get(id)
                    .map(|e| e.info.name.clone())
                    .unwrap_or_default()
            };
            let left_out: Vec<LeftOut> = states
                .iter()
                .filter(|st| st.status != AreaStatus::Unreadable && !st.member)
                .map(|st| LeftOut {
                    id: st.id,
                    name: named(st.id),
                })
                .collect();
            let unreadable: Vec<Failed> = s
                .photos
                .infos()
                .into_iter()
                .filter_map(|i| {
                    Some(Failed {
                        id: i.id,
                        name: i.name,
                        reason: i.problem?.message(),
                    })
                })
                .collect();
            (
                states.iter().any(|st| st.blocks_removal(open)),
                todo,
                finished,
                left_out,
                unreadable,
            )
        };
        if blocked {
            return Err(AppError::NotReady);
        }
        if todo.is_empty() && finished.is_empty() {
            return Err(AppError::NothingToRemove);
        }
        if todo.is_empty() {
            return Ok(RemoveReport {
                done: finished,
                failed: unreadable,
                left_out,
                cancelled: false,
            });
        }
        let paths: Vec<PathBuf> = todo.iter().map(|t| t.path.clone()).collect();
        let dirs = output_dirs(&paths, &rule).map_err(|_| AppError::NoWritableOutput)?;
        let mut cleaned: Vec<&PathBuf> = dirs.values().collect();
        cleaned.sort();
        cleaned.dedup();
        for dir in cleaned {
            remove_stale_partials(dir);
        }

        let mut report = RemoveReport {
            done: finished,
            failed: unreadable,
            left_out,
            cancelled: false,
        };
        let engine = match self.ensure_models_in(&job, &[ModelKind::Lama], LoadPurpose::Remover, ev)
        {
            Err(e) if is_cancelled(&e) => {
                report.cancelled = true;
                return Ok(report);
            }
            other => other?,
        };
        self.announce_fallback(&engine, ev);
        let work_long_side = detail.work_long_side();
        let total = todo.len() as u32;
        let mut eta = Eta::new(todo.len());
        for (i, t) in todo.iter().enumerate() {
            if job.check().is_err() {
                report.cancelled = true;
                break;
            }
            let done = i as u32;
            let start = Instant::now();
            let removing = |photo_fraction: f32, seconds_left: Option<u32>| {
                ev(ProgressEvent::Removing {
                    done,
                    total,
                    photo: t.id,
                    photo_fraction,
                    seconds_left,
                })
            };
            removing(0.0, eta.seconds_left(0.0));
            let on_fraction = |f: f32| removing(f, eta.seconds_left(start.elapsed().as_secs_f32()));
            let progress = Progress {
                on_fraction: &on_fraction,
                cancel: job.flag(),
            };
            let out_dir = t.path.parent().and_then(|p| dirs.get(p));
            let photo = load(&t.path).map_err(AppError::from);
            let loaded = photo.is_ok();
            if let Err(e) = &photo
                && let Some(p) = Problem::of(e)
            {
                self.session().photos.set_problem(t.id, p);
            }
            let result = photo.and_then(|photo| {
                let out_dir = out_dir.ok_or(AppError::NoWritableOutput)?;
                let (w, h) = photo.rgb.dimensions();
                let area = mask::binarize(&t.area.resized(w, h));
                Ok(remove_one(
                    &engine,
                    &photo,
                    &area,
                    out_dir,
                    Some(work_long_side),
                    progress,
                )?)
            });
            self.announce_fallback(&engine, ev);
            match result {
                Ok(saved) => {
                    if let Some(d) = self.session().data.get_mut(&t.id) {
                        d.result = Some(saved.clone());
                        d.result_made = Some(Made {
                            area: t.area.clone(),
                            output: rule.clone(),
                            detail,
                        });
                    }
                    report.done.push(Done {
                        id: t.id,
                        output: saved.to_string_lossy().into_owned(),
                    });
                }
                Err(e) if is_cancelled(&e) => {
                    report.cancelled = true;
                    break;
                }
                Err(e) => report.failed.push(Failed {
                    id: t.id,
                    name: t.name.clone(),
                    reason: user_message(&e),
                }),
            }
            // A photo that could not be read took no time and says nothing about the others.
            if loaded {
                eta.photo_done(start.elapsed().as_secs_f32());
            } else {
                eta.photo_skipped();
            }
            ev(ProgressEvent::Removing {
                done: done + 1,
                total,
                photo: t.id,
                photo_fraction: 1.0,
                seconds_left: eta.seconds_left(0.0),
            });
        }
        Ok(report)
    }

    /// The photo `id` as it was (JPEG at edit size), for the before/after view.
    pub fn original_image(&self, id: PhotoId) -> Result<Vec<u8>> {
        self.edit_image(id)
    }

    /// What the last run saved for photo `id` (JPEG at edit size), for the before/after view.
    pub fn result_image(&self, id: PhotoId) -> Result<Vec<u8>> {
        let path = {
            let s = self.session();
            s.photos.get(id).ok_or_else(|| no_photo(id))?;
            s.data
                .get(&id)
                .and_then(|d| d.result.clone())
                .ok_or_else(|| not_found("no result for this photo"))?
        };
        Ok(jpeg(&images::edit_image(&load(&path)?), IMAGE_QUALITY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu() -> Option<Backend> {
        Some(Backend::Gpu {
            name: "Core ML".into(),
            share: None,
        })
    }

    fn failed() -> Option<Backend> {
        Some(Backend::Cpu {
            reason: CpuReason::RunFailed("boom".into()),
        })
    }

    fn report(lama: Option<Backend>) -> DeviceReport {
        DeviceReport {
            segmenter: gpu(),
            lama,
            face: None,
        }
    }

    #[test]
    fn fallbacks_are_run_failures_only() {
        assert!(fallbacks(&report(gpu())).is_empty());
        assert_eq!(
            fallbacks(&report(failed())),
            [(ModelKind::Lama, "boom".to_string())]
        );
        let cpu = Some(Backend::Cpu {
            reason: CpuReason::NotRequested,
        });
        assert!(fallbacks(&report(cpu)).is_empty());
    }

    #[test]
    fn report_is_camel_case() {
        let r = RemoveReport {
            done: vec![Done {
                id: 1,
                output: "/a/b.jpg".into(),
            }],
            failed: vec![],
            left_out: vec![],
            cancelled: false,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(
            json,
            r#"{"done":[{"id":1,"output":"/a/b.jpg"}],"failed":[],"leftOut":[],"cancelled":false}"#
        );
    }
}
