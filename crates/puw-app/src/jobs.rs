//! Long jobs: one at a time, each with its own cancel flag, and the progress events they send.
//!
//! The interface gives each job it starts a ticket (a number) and cancels by ticket, so a stop
//! meant for one job never reaches the next one. A job started while a cancelled one is still
//! finishing (a model load cannot be interrupted, say) waits for it instead of failing with
//! [`AppError::Busy`]; only a job that runs on, not cancelled, makes the next one busy.

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
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

thread_local! {
    /// The ticket of the call running on this thread, see [`with_ticket`].
    static TICKET: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Runs `f` (an [`crate::AppCore`] call) as the job with `ticket`: a job it starts can be
/// cancelled by that ticket ([`crate::AppCore::cancel_ticket`]).
pub fn with_ticket<T>(ticket: Option<u64>, f: impl FnOnce() -> T) -> T {
    struct Reset(Option<u64>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TICKET.with(|t| t.set(self.0));
        }
    }
    let _reset = Reset(TICKET.with(|t| t.replace(ticket)));
    f()
}

/// Tickets cancelled before their job started that are remembered.
const EARLY_CANCELS: usize = 32;

#[derive(Debug)]
struct Running {
    cancel: Arc<AtomicBool>,
    ticket: Option<u64>,
}

#[derive(Debug, Default)]
struct SlotState {
    current: Option<Running>,
    /// Tickets cancelled when no job of theirs ran: a job that starts with one of them stops
    /// at once (its cancel overtook it).
    early: VecDeque<u64>,
}

/// The one place a long job may run. The lock is held only to start, cancel or end a job (and
/// while waiting for a cancelled one to end).
#[derive(Debug, Default)]
pub(crate) struct JobSlot {
    state: Mutex<SlotState>,
    ended: Condvar,
}

/// A running job; the slot is free again when this is dropped.
pub(crate) struct Job<'a> {
    slot: &'a JobSlot,
    cancel: Arc<AtomicBool>,
}

impl JobSlot {
    /// Starts a job with the ticket of this call ([`with_ticket`]). While a cancelled job is
    /// still finishing it waits for that job's end; while one runs that was not cancelled it
    /// fails with [`AppError::Busy`]. A ticket that was cancelled before its job started gives
    /// [`Error::Cancelled`].
    pub fn start(&self) -> Result<Job<'_>> {
        let ticket = TICKET.with(Cell::get);
        let mut state = lock(&self.state);
        loop {
            if let Some(t) = ticket
                && let Some(i) = state.early.iter().position(|&e| e == t)
            {
                state.early.remove(i);
                return Err(Error::Cancelled.into());
            }
            match &state.current {
                None => break,
                Some(r) if r.cancel.load(Ordering::Relaxed) => {
                    state = self
                        .ended
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                Some(_) => return Err(AppError::Busy),
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.current = Some(Running {
            cancel: cancel.clone(),
            ticket,
        });
        Ok(Job { slot: self, cancel })
    }

    /// Asks the running job, whatever it is, to stop.
    pub fn cancel(&self) {
        if let Some(r) = lock(&self.state).current.as_ref() {
            r.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Asks the job with `ticket` to stop: the running one when it has that ticket, else the
    /// job with that ticket when it starts later. Any other job runs on.
    pub fn cancel_ticket(&self, ticket: u64) {
        let mut state = lock(&self.state);
        match &state.current {
            Some(r) if r.ticket == Some(ticket) => r.cancel.store(true, Ordering::Relaxed),
            _ => {
                if state.early.len() == EARLY_CANCELS {
                    state.early.pop_front();
                }
                state.early.push_back(ticket);
            }
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
        lock(&self.slot.state).current = None;
        self.slot.ended.notify_all();
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
    fn a_cancelled_job_is_waited_for_not_busy() {
        let slot = Arc::new(JobSlot::default());
        let job = slot.start().unwrap();
        slot.cancel();
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|s| {
            s.spawn(|| {
                let next = slot.start().map(|_| ());
                tx.send(next.is_ok()).unwrap();
            });
            // The new job waits while the cancelled one finishes.
            assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
            drop(job);
            assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
        });
    }

    #[test]
    fn cancel_by_ticket_reaches_only_that_job() {
        let slot = JobSlot::default();
        // Job 1 ran and ended; its late cancel must not stop job 2.
        drop(with_ticket(Some(1), || slot.start()).unwrap());
        let two = with_ticket(Some(2), || slot.start()).unwrap();
        slot.cancel_ticket(1);
        assert!(two.check().is_ok());
        slot.cancel_ticket(2);
        assert!(matches!(two.check(), Err(AppError::Core(Error::Cancelled))));
        drop(two);
        // A cancel that overtakes its job's start stops it at once, and only it.
        slot.cancel_ticket(4);
        let three = with_ticket(Some(3), || slot.start()).unwrap();
        assert!(three.check().is_ok());
        drop(three);
        assert!(matches!(
            with_ticket(Some(4), || slot.start()),
            Err(AppError::Core(Error::Cancelled))
        ));
        // The ticket only counts for the call it wraps.
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
