//! Application state shared by all commands: open sessions, running queries,
//! background jobs, and paths for config and caches.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use dbcore::{Error, Session};

use crate::jobs::Job;
use crate::runner::RunningQuery;

type Result<T> = std::result::Result<T, Error>;

/// Finished queries and jobs the UI forgot to release are dropped after this long.
const KEEP_FINISHED: Duration = Duration::from_secs(30 * 60);

pub struct Paths {
    /// `connections.json` lives here.
    pub config_dir: PathBuf,
    /// Schema snapshots live under here.
    pub cache_dir: PathBuf,
}

#[derive(Default)]
pub struct AppState {
    paths: OnceLock<Paths>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// One lock per connection id, so two simultaneous opens connect once.
    opening: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    queries: Mutex<HashMap<String, Arc<RunningQuery>>>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    /// Serializes read-modify-write cycles of `connections.json`.
    pub connections_lock: tokio::sync::Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AppState {
    /// Called once from `setup`.
    pub fn init(&self, config_dir: PathBuf, cache_dir: PathBuf) {
        let _ = self.paths.set(Paths {
            config_dir,
            cache_dir,
        });
    }

    pub fn paths(&self) -> Result<&Paths> {
        self.paths
            .get()
            .ok_or_else(|| Error::Other("The application is still starting up.".into()))
    }

    // --- sessions ------------------------------------------------------------

    pub fn session(&self, id: &str) -> Result<Arc<Session>> {
        lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound("This connection is not open.".into()))
    }

    pub fn find_session(&self, id: &str) -> Option<Arc<Session>> {
        lock(&self.sessions).get(id).cloned()
    }

    pub fn insert_session(&self, id: String, session: Session) -> Arc<Session> {
        let session = Arc::new(session);
        lock(&self.sessions).insert(id, session.clone());
        session
    }

    pub fn open_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.opening)
            .entry(id.to_owned())
            .or_default()
            .clone()
    }

    /// Cancels the session's queries, then closes the driver and the SSH tunnel.
    pub async fn close_session(&self, id: &str) {
        for query in self.queries_of(id) {
            query.cancel().await;
            lock(&self.queries).remove(&query.id);
        }
        let session = lock(&self.sessions).remove(id);
        if let Some(session) = session {
            match Arc::try_unwrap(session) {
                Ok(session) => session.close().await,
                // A command still holds the session; closing the driver ends its work,
                // and the tunnel stops when the last reference drops.
                Err(shared) => shared.driver.close().await,
            }
        }
    }

    // --- queries -------------------------------------------------------------

    pub fn add_query(&self, query: Arc<RunningQuery>) {
        let mut queries = lock(&self.queries);
        queries.retain(|_, q| !(q.is_finished() && q.age() > KEEP_FINISHED));
        queries.insert(query.id.clone(), query);
    }

    pub fn query(&self, id: &str) -> Result<Arc<RunningQuery>> {
        lock(&self.queries)
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound("This query was closed.".into()))
    }

    pub fn remove_query(&self, id: &str) -> Option<Arc<RunningQuery>> {
        lock(&self.queries).remove(id)
    }

    fn queries_of(&self, session_id: &str) -> Vec<Arc<RunningQuery>> {
        lock(&self.queries)
            .values()
            .filter(|q| q.session_id == session_id)
            .cloned()
            .collect()
    }

    // --- jobs ------------------------------------------------------------------

    pub fn add_job(&self, id: String, job: Arc<Job>) {
        let mut jobs = lock(&self.jobs);
        jobs.retain(|_, j| j.finished_at().is_none_or(|t| t.elapsed() < KEEP_FINISHED));
        jobs.insert(id, job);
    }

    pub fn job(&self, id: &str) -> Result<Arc<Job>> {
        lock(&self.jobs)
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound("Unknown job.".into()))
    }
}
