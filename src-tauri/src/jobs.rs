//! Background export/import jobs: progress per job id plus a cancellation token.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use dbcore::Error;
use dbcore::export::{CancellationToken, ProgressFn};
use dbcore::model::JobProgress;

pub struct Job {
    progress: Mutex<JobProgress>,
    finished_at: Mutex<Option<Instant>>,
    pub cancel: CancellationToken,
}

impl Job {
    pub fn new() -> Arc<Job> {
        Arc::new(Job {
            progress: Mutex::new(JobProgress {
                rows: 0,
                bytes: 0,
                done: false,
                cancelled: false,
                error: None,
                warning: None,
            }),
            finished_at: Mutex::new(None),
            cancel: CancellationToken::new(),
        })
    }

    pub fn snapshot(&self) -> JobProgress {
        self.progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn finished_at(&self) -> Option<Instant> {
        *self
            .finished_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The callback handed to `dbcore::export`.
    pub fn progress_fn(self: &Arc<Self>) -> ProgressFn {
        let job = self.clone();
        Arc::new(move |p| job.set(p.clone()))
    }

    fn set(&self, progress: JobProgress) {
        let done = progress.done;
        *self.progress.lock().unwrap_or_else(PoisonError::into_inner) = progress;
        if done {
            *self
                .finished_at
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        }
    }

    fn fail(&self, message: String) {
        let mut p = self.snapshot();
        p.done = true;
        if p.error.is_none() {
            p.error = Some(message);
        }
        self.set(p);
    }
}

/// Runs `work` in the background and records its outcome on `job`, also when it
/// fails or panics, so `job_status` always ends in a `done` state.
pub fn spawn<F>(job: Arc<Job>, work: F)
where
    F: Future<Output = Result<JobProgress, Error>> + Send + 'static,
{
    let worker = tokio::spawn(work);
    tokio::spawn(async move {
        match worker.await {
            Ok(Ok(progress)) => job.set(progress),
            Ok(Err(e)) => job.fail(e.to_string()),
            Err(_) => job.fail("The job stopped unexpectedly.".into()),
        }
    });
}
