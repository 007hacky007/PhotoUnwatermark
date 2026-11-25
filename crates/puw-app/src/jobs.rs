//! Long jobs: one at a time, each with its own cancel flag, and the progress events they send.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use puw_core::Error;
use serde::Serialize;

use crate::photos::PhotoId;
use crate::{AppError, Result};

/// Which models are loading, for the sentence under the bar: "Getting the watermark finder
/// ready..." or "Getting the watermark remover ready...".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoadPurpose {
    Finder,
    Remover,
}

/// What a long job is doing. Fractions run from 0.0 to 1.0.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "phase",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProgressEvent {
    LoadingModels {
        fraction: f32,
        which: LoadPurpose,
    },
    Reading {
        fraction: f32,
    },
    Looking {
        done: u32,
        total: u32,
    },
    Matching {
        done: u32,
        total: u32,
    },
    Removing {
        done: u32,
        total: u32,
        photo: PhotoId,
        photo_fraction: f32,
        seconds_left: Option<u32>,
    },
    Preview {
        fraction: f32,
    },
    FellBackToProcessor {
        reason: String,
    },
}

/// Where a job sends its progress. It may be called from more than one thread, but never from
/// two at the same time, and never while the session is locked.
pub type Events<'a> = &'a (dyn Fn(ProgressEvent) + Sync);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The one place a long job may run. The lock is held only to start, cancel or end a job.
#[derive(Debug, Default)]
pub(crate) struct JobSlot {
    current: Mutex<Option<Arc<AtomicBool>>>,
}

/// A running job; the slot is free again when this is dropped.
pub(crate) struct Job<'a> {
    slot: &'a JobSlot,
    cancel: Arc<AtomicBool>,
}

impl JobSlot {
    /// Starts a job, or fails with [`AppError::Busy`] while another one runs.
    pub fn start(&self) -> Result<Job<'_>> {
        let mut current = lock(&self.current);
        if current.is_some() {
            return Err(AppError::Busy);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *current = Some(cancel.clone());
        Ok(Job { slot: self, cancel })
    }

    /// Asks the running job, if any, to stop.
    pub fn cancel(&self) {
        if let Some(flag) = lock(&self.current).as_ref() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

impl Job<'_> {
    pub fn flag(&self) -> &AtomicBool {
        &self.cancel
    }

    /// [`Error::Cancelled`] once the job was asked to stop.
    pub fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled.into());
        }
        Ok(())
    }
}

impl Drop for Job<'_> {
    fn drop(&mut self) {
        *lock(&self.slot.current) = None;
    }
}

/// How often a model load reports its estimated progress.
pub(crate) const TICK: Duration = Duration::from_millis(200);

/// Runs `work` on this thread while another thread calls `tick` with the seconds since the start
/// every [`TICK`]. The ticker has stopped when this returns, also when `work` panics.
pub(crate) fn with_ticker<T>(tick: &(dyn Fn(f32) + Sync), work: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    std::thread::scope(|s| {
        let (stop, stopped) = mpsc::channel::<()>();
        s.spawn(move || {
            while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(TICK) {
                tick(start.elapsed().as_secs_f32());
            }
        });
        let out = work();
        drop(stop);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_job_at_a_time() {
        let slot = JobSlot::default();
        let job = slot.start().unwrap();
        assert!(matches!(slot.start(), Err(AppError::Busy)));
        drop(job);
        let job = slot.start().unwrap();
        assert!(job.check().is_ok());
    }

    #[test]
    fn cancel_reaches_only_the_running_job() {
        let slot = JobSlot::default();
        slot.cancel();
        let job = slot.start().unwrap();
        assert!(job.check().is_ok());
        slot.cancel();
        assert!(matches!(job.check(), Err(AppError::Core(Error::Cancelled))));
        drop(job);
        assert!(slot.start().unwrap().check().is_ok());
    }

    #[test]
    fn ticker_ticks_while_working_and_stops() {
        let ticks = Mutex::new(Vec::new());
        let out = with_ticker(&|s| lock(&ticks).push(s), || {
            std::thread::sleep(Duration::from_millis(700));
            7
        });
        assert_eq!(out, 7);
        let seen = lock(&ticks).clone();
        assert!((2..=4).contains(&seen.len()), "{seen:?}");
        assert!(seen.windows(2).all(|p| p[0] < p[1]));
        std::thread::sleep(TICK * 2);
        assert_eq!(lock(&ticks).len(), seen.len());
    }

    #[test]
    fn events_serialize_with_phase() {
        let json = serde_json::to_string(&ProgressEvent::LoadingModels {
            fraction: 0.5,
            which: LoadPurpose::Finder,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"phase":"loadingModels","fraction":0.5,"which":"finder"}"#
        );
        let json = serde_json::to_string(&ProgressEvent::Removing {
            done: 1,
            total: 2,
            photo: 3,
            photo_fraction: 0.25,
            seconds_left: None,
        })
        .unwrap();
        assert!(json.contains(r#""phase":"removing""#) && json.contains("photoFraction"));
        assert!(json.contains("secondsLeft"));
    }
}
