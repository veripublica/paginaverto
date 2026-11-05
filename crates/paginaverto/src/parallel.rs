//! Runs a step over a document's pages on several threads. The result keeps
//! page order, so the thread count cannot change the output.

use std::any::Any;
use std::panic::resume_unwind;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{fmt, thread};

use lopdf::ObjectId;

use crate::Error;

/// A worker's stack: what a main thread has, since lopdf's parser recurses and
/// a stack overflow cannot be caught.
const WORKER_STACK_BYTES: usize = 8 << 20;

/// Asks a running [`read`](crate::read) to stop. Clones share one flag; hand a
/// clone to another thread and call [`cancel`](Cancel::cancel) there. It is
/// checked between pages, so a page already being read finishes first. The
/// read then returns [`Error::Cancelled`].
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A flag that has not been raised.
    pub fn new() -> Cancel {
        Cancel::default()
    }

    /// Raise the flag. It cannot be lowered: use a new `Cancel` per read.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Has the flag been raised?
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// One page finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageDone {
    /// The page that finished, from 1. Pages finish in any order.
    pub page: u32,
    /// How many pages have finished, this one included. Only ever grows.
    pub done: usize,
    /// How many pages will be read.
    pub total: usize,
}

/// Called after each page is read: possibly on a worker thread, but never on
/// two threads at once. Keep it short, since the other threads' reports wait.
#[derive(Clone)]
pub struct Progress(Arc<dyn Fn(PageDone) + Send + Sync>);

impl Progress {
    /// Report to this function.
    pub fn new(report: impl Fn(PageDone) + Send + Sync + 'static) -> Progress {
        Progress(Arc::new(report))
    }
}

impl fmt::Debug for Progress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Progress(..)")
    }
}

/// Every core but two, and at least one. One where threads cannot be counted,
/// as in a browser.
fn default_threads() -> usize {
    thread::available_parallelism()
        .map_or(1, |n| n.get().saturating_sub(2))
        .max(1)
}

pub(crate) struct Pool<'a> {
    pub(crate) threads: Option<usize>,
    pub(crate) cancel: &'a Cancel,
}

impl Pool<'_> {
    /// This pool, with at most `max` threads unless the caller chose a number.
    #[cfg_attr(not(feature = "ocr"), allow(dead_code))]
    pub(crate) fn at_most(&self, max: usize) -> Pool<'_> {
        Pool {
            threads: self.threads.or(Some(default_threads().min(max))),
            cancel: self.cancel,
        }
    }

    /// `f` of every page, in page order, or [`Error::Cancelled`].
    pub(crate) fn map<T: Send>(
        &self,
        pages: &[(u32, ObjectId)],
        progress: Option<&Progress>,
        f: impl Fn(u32, ObjectId) -> T + Sync,
    ) -> Result<Vec<T>, Error> {
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let done = Mutex::new(0usize);
        let work = || {
            let _stop_on_panic = StopOnPanic(&stop);
            let mut out = Vec::new();
            while !stop.load(Ordering::Relaxed) && !self.cancel.is_cancelled() {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(page, id)) = pages.get(i) else {
                    break;
                };
                out.push((i, f(page, id)));
                if let Some(progress) = progress {
                    // Held across the call: reports never overlap, and `done`
                    // only grows. A panicking reporter is caught at `read`.
                    let mut done = done.lock().unwrap_or_else(|e| e.into_inner());
                    *done += 1;
                    (progress.0)(PageDone {
                        page,
                        done: *done,
                        total: pages.len(),
                    });
                }
            }
            out
        };

        let threads = self
            .threads
            .unwrap_or_else(default_threads)
            .clamp(1, pages.len().max(1));
        let mut results = thread::scope(|scope| {
            let helpers: Vec<_> = (1..threads)
                .map_while(|_| {
                    // A platform without threads simply leaves it all to this one.
                    thread::Builder::new()
                        .stack_size(WORKER_STACK_BYTES)
                        .spawn_scoped(scope, work)
                        .ok()
                })
                .collect();
            let mut all = work();
            for helper in helpers {
                match helper.join() {
                    Ok(part) => all.extend(part),
                    Err(payload) => rethrow(payload),
                }
            }
            all
        });

        if self.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        results.sort_unstable_by_key(|(i, _)| *i);
        Ok(results.into_iter().map(|(_, value)| value).collect())
    }
}

/// Keeps the panic's own message, which `read` turns into [`Error::Internal`].
fn rethrow(payload: Box<dyn Any + Send>) -> ! {
    resume_unwind(payload)
}

/// Stops the other workers when one panics.
struct StopOnPanic<'a>(&'a AtomicBool);

impl Drop for StopOnPanic<'_> {
    fn drop(&mut self) {
        if thread::panicking() {
            self.0.store(true, Ordering::Relaxed);
        }
    }
}
