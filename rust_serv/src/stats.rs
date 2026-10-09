//! Per-server counters shared by all connection jobs and the /stats handler.
use crate::http::response::{Response, Status};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
pub struct Stats {
    pub(crate) accepted: AtomicUsize,
    pub(crate) queued: AtomicUsize,
    pub(crate) active: AtomicUsize,
    pub(crate) completed: AtomicUsize,
    pub(crate) rejected: AtomicUsize,
    pub(crate) requests: AtomicUsize,
    pub(crate) responses: AtomicUsize,
    pub(crate) errors: AtomicUsize,
    pub(crate) panics: AtomicUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub accepted: usize,
    pub queued: usize,
    pub active: usize,
    pub completed: usize,
    pub rejected: usize,
    pub requests: usize,
    pub responses: usize,
    pub errors: usize,
    pub panics: usize,
}

impl Stats {
    /// Individually atomic counters; a live snapshot is not an atomic transaction.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            accepted: self.accepted.load(Ordering::Relaxed),
            queued: self.queued.load(Ordering::Relaxed),
            active: self.active.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            requests: self.requests.load(Ordering::Relaxed),
            responses: self.responses.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            panics: self.panics.load(Ordering::Relaxed),
        }
    }

    pub fn response(&self) -> Response {
        let s = self.snapshot();
        Response::new(Status::Ok).header("Content-Type", "application/json")
            .header("Cache-Control", "no-store")
            .body(format!("{{\"accepted\":{},\"queued\":{},\"active\":{},\"completed\":{},\"rejected\":{},\"requests\":{},\"responses\":{},\"errors\":{},\"panics\":{}}}\n",
                s.accepted, s.queued, s.active, s.completed, s.rejected, s.requests, s.responses, s.errors, s.panics))
    }
}

/// Admission owns a ticket even before the job starts. Dropping a rejected job
/// corrects queued automatically; unwinding a running job corrects active.
pub(crate) struct Ticket {
    stats: Arc<Stats>,
    started: bool,
}

impl Ticket {
    pub(crate) fn new(stats: Arc<Stats>) -> Self {
        stats.accepted.fetch_add(1, Ordering::Relaxed);
        stats.queued.fetch_add(1, Ordering::Relaxed);
        Self {
            stats,
            started: false,
        }
    }
    pub(crate) fn start(&mut self) {
        self.stats.queued.fetch_sub(1, Ordering::Relaxed);
        self.stats.active.fetch_add(1, Ordering::Relaxed);
        self.started = true;
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if self.started {
            self.stats.active.fetch_sub(1, Ordering::Relaxed);
            self.stats.completed.fetch_add(1, Ordering::Relaxed);
        } else {
            self.stats.queued.fetch_sub(1, Ordering::Relaxed);
            self.stats.rejected.fetch_add(1, Ordering::Relaxed);
        }
    }
}
