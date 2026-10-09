use rust_serv::thread_pool::ThreadPool;
use std::{
    io,
    sync::{
        Arc, Condvar, Mutex, RwLock,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

fn gate() -> Arc<(Mutex<bool>, Condvar)> {
    Arc::new((Mutex::new(false), Condvar::new()))
}
fn wait(g: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, wake) = &**g;
    let mut ready = lock.lock().unwrap();
    while !*ready {
        ready = wake.wait(ready).unwrap();
    }
}
fn release(g: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, wake) = &**g;
    *lock.lock().unwrap() = true;
    wake.notify_all();
}
#[test]
fn workers_execute_concurrently_without_holding_receiver_lock() {
    let mut pool = ThreadPool::new(2).unwrap();
    let gate = gate();
    let (tx, rx) = mpsc::channel();
    for _ in 0..2 {
        let tx = tx.clone();
        let gate = gate.clone();
        pool.execute(move || {
            tx.send(()).unwrap();
            wait(&gate);
        })
        .unwrap();
    }
    let first = rx.recv_timeout(Duration::from_secs(2));
    let second = rx.recv_timeout(Duration::from_secs(2));
    release(&gate); // Always release before assertions, including on a regression.
    pool.shutdown().unwrap();
    assert!(
        first.is_ok() && second.is_ok(),
        "receiver mutex serialized the jobs"
    );
}
#[test]
fn queue_is_bounded_and_overflow_does_not_block_submission() {
    let mut pool = ThreadPool::with_capacity(1, 1).unwrap();
    let gate = gate();
    let worker_gate = gate.clone();
    let (tx, rx) = mpsc::channel();
    pool.execute(move || {
        tx.send(()).unwrap();
        wait(&worker_gate);
    })
    .unwrap();
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    pool.execute(|| {}).unwrap();
    let overflow = pool.execute(|| {});
    release(&gate);
    pool.shutdown().unwrap();
    assert_eq!(overflow.unwrap_err().kind(), io::ErrorKind::WouldBlock);
}
#[test]
fn panic_is_contained_and_next_job_runs_on_same_worker() {
    let mut pool = ThreadPool::new(1).unwrap();
    let shared = Arc::new(Mutex::new(0));
    let writer = shared.clone();
    pool.execute(move || {
        let mut guard = writer.lock().unwrap();
        *guard = 1;
        panic!("intentional test panic");
    })
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = shared.clone();
    pool.execute(move || {
        // The user mutex is poisoned; the pool's receiver mutex is not.
        assert!(reader.is_poisoned());
        let mut guard = reader.lock().unwrap_or_else(|error| error.into_inner());
        *guard += 1;
        tx.send(*guard).unwrap();
    })
    .unwrap();
    let result = rx.recv_timeout(Duration::from_secs(2));
    pool.shutdown().unwrap();
    assert_eq!(result.unwrap(), 2);
    assert_eq!(pool.panics(), 1);
}
#[test]
fn shutdown_drains_all_jobs_is_idempotent_and_rejects_new_work() {
    let mut pool = ThreadPool::with_capacity(4, 256).unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    for _ in 0..100 {
        let c = counter.clone();
        pool.execute(move || {
            c.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
    }
    pool.shutdown().unwrap();
    pool.shutdown().unwrap();
    assert_eq!(counter.load(Ordering::Relaxed), 100);
    assert_eq!(
        pool.execute(|| {}).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}
#[test]
fn drop_joins_and_executes_queued_fn_once_jobs() {
    let count = Arc::new(AtomicUsize::new(0));
    {
        let pool = ThreadPool::with_capacity(2, 32).unwrap();
        for _ in 0..20 {
            let c = count.clone();
            let owned = String::from("consumed");
            pool.execute(move || {
                drop(owned);
                c.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
        }
    }
    assert_eq!(count.load(Ordering::Relaxed), 20);
}
#[test]
fn rejects_zero_workers_and_zero_capacity() {
    assert!(ThreadPool::new(0).is_err());
    assert!(ThreadPool::with_capacity(1, 0).is_err());
}
#[test]
fn rwlock_can_share_read_mostly_state_between_jobs() {
    let mut pool = ThreadPool::new(2).unwrap();
    let config = Arc::new(RwLock::new(String::from("initial")));
    let writer = config.clone();
    let (tx, rx) = mpsc::channel();
    pool.execute(move || {
        *writer.write().unwrap() = "updated".into();
        tx.send(()).unwrap();
    })
    .unwrap();
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let (tx, rx) = mpsc::channel();
    for _ in 0..2 {
        let reader = config.clone();
        let tx = tx.clone();
        pool.execute(move || {
            tx.send(reader.read().unwrap().clone()).unwrap();
        })
        .unwrap();
    }
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "updated");
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "updated");
    pool.shutdown().unwrap();
}
