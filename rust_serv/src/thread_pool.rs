//! Fixed workers with a bounded mpsc queue. One job may own an entire connection.
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
};

pub type Job = Box<dyn FnOnce() + Send + 'static>;

pub struct ThreadPool {
    sender: Option<SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
    panics: Arc<AtomicUsize>,
}

impl ThreadPool {
    pub fn new(workers: usize) -> io::Result<Self> {
        Self::with_capacity(workers, 64)
    }

    pub fn with_capacity(workers: usize, capacity: usize) -> io::Result<Self> {
        if workers == 0 || capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "workers and queue capacity must be positive",
            ));
        }
        let (sender, receiver) = mpsc::sync_channel::<Job>(capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        let panics = Arc::new(AtomicUsize::new(0));
        let mut pool = Self {
            sender: Some(sender),
            workers: Vec::new(),
            panics,
        };
        for id in 0..workers {
            let receiver = Arc::clone(&receiver);
            let panics = Arc::clone(&pool.panics);
            let worker = thread::Builder::new()
                .name(format!("http-worker-{id}"))
                .spawn(move || {
                    loop {
                        // Do NOT write `while let Ok(job) = receiver.lock().unwrap().recv()`:
                        // that temporary guard can live through the whole loop body.
                        let received = {
                            let guard = receiver
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            guard.recv()
                        }; // Unlock before running user code or socket I/O.
                        let Ok(job) = received else {
                            break;
                        };
                        if catch_unwind(AssertUnwindSafe(job)).is_err() {
                            panics.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })?;
            pool.workers.push(worker);
        }
        // If spawning failed, pool's Drop closes the channel and joins the workers already created.
        Ok(pool)
    }

    /// Nonblocking admission: a full queue rejects and drops the supplied job.
    pub fn execute(&self, job: impl FnOnce() + Send + 'static) -> io::Result<()> {
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "pool is shut down"))?;
        match sender.try_send(Box::new(job)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "pool queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "pool workers disconnected",
            )),
        }
    }

    pub fn panics(&self) -> usize {
        self.panics.load(Ordering::Relaxed)
    }

    /// Close admission, drain queued jobs, and join all workers. Idempotent.
    pub fn shutdown(&mut self) -> io::Result<()> {
        drop(self.sender.take()); // Disconnect BEFORE joining: recv must wake up.
        let mut failed = false;
        for worker in self.workers.drain(..) {
            failed |= worker.join().is_err();
        }
        if failed {
            Err(io::Error::other("worker panicked outside a job"))
        } else {
            Ok(())
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
