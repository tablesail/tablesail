//! Application state shared by all commands: open sessions, running queries,
//! background jobs, and paths for config and caches.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use dbcore::transaction::Transactions;
use dbcore::{Error, Session};

use crate::jobs::Job;
use crate::runner::RunningQuery;

type Result<T> = std::result::Result<T, Error>;

/// Finished queries and jobs the UI forgot to release are dropped after this long.
const KEEP_FINISHED: Duration = Duration::from_secs(30 * 60);

/// Quitting waits at most this long for connections to roll back and close.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

/// Closing a connection waits at most this long for its cancelled jobs to wrap up.
const JOB_WRAP_UP: Duration = Duration::from_secs(3);

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
    /// Manual transactions, each on a dedicated connection of its session.
    pub txs: Transactions,
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

    /// Cancels the session's queries and jobs, rolls back its transactions, then closes the
    /// driver and the SSH tunnel.
    pub async fn close_session(&self, id: &str) {
        for query in self.queries_of(id) {
            query.abort().await;
            lock(&self.queries).remove(&query.id);
        }
        self.cancel_jobs_of(id).await;
        self.txs.rollback_session(id).await;
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

    /// Quitting: closes every session, which rolls back every open transaction.
    pub async fn shutdown(&self) {
        let ids: Vec<String> = lock(&self.sessions).keys().cloned().collect();
        let closing = async {
            for id in &ids {
                self.close_session(id).await;
            }
            // Transactions of sessions that were closed some other way.
            self.txs.rollback_all().await;
        };
        let _ = tokio::time::timeout(SHUTDOWN_BUDGET, closing).await;
    }

    // --- transactions ----------------------------------------------------------

    fn queries_of_tx(&self, tx_id: &str) -> Vec<Arc<RunningQuery>> {
        lock(&self.queries)
            .values()
            .filter(|q| q.tx_id.as_deref() == Some(tx_id))
            .cloned()
            .collect()
    }

    /// Makes the earlier queries of a transaction let go of its connection. A result being
    /// scrolled holds the connection (one statement at a time), so a new statement, or the
    /// commit, would wait for it forever; reading stops where it is and the rows already
    /// loaded stay available. A statement that is still running is left alone: whatever comes
    /// next queues behind it.
    pub async fn settle_queries_of_tx(&self, tx_id: &str) {
        for query in self.queries_of_tx(tx_id) {
            query.settle().await;
        }
    }

    /// Stops the queries of a transaction that is being rolled back, running statements included.
    pub async fn abort_queries_of_tx(&self, tx_id: &str) {
        for query in self.queries_of_tx(tx_id) {
            query.abort().await;
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

    /// Cancels the running jobs that belong to the session (a dump import runs on a connection
    /// of its own, which closing the session does not reach) and gives them a moment to roll
    /// back and close that connection.
    async fn cancel_jobs_of(&self, session_id: &str) {
        let running: Vec<Arc<Job>> = lock(&self.jobs)
            .values()
            .filter(|j| j.session_id() == Some(session_id) && j.finished_at().is_none())
            .cloned()
            .collect();
        for job in &running {
            job.cancel.cancel();
        }
        let _ = tokio::time::timeout(JOB_WRAP_UP, async {
            while running.iter().any(|j| j.finished_at().is_none()) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RunOptions;
    use dbcore::driver::{CursorEvent, Driver};
    use dbcore::model::{ConnectionConfig, Secrets, StatementKind, StatementSpan};

    /// A SQLite database file in a scratch directory that is removed afterwards.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Scratch {
            let dir =
                std::env::temp_dir().join(format!("tablesail-state-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::File::create(dir.join("test.sqlite")).unwrap();
            Scratch(dir)
        }

        async fn open(&self) -> Session {
            let path = self.0.join("test.sqlite");
            let config: ConnectionConfig = serde_json::from_value(serde_json::json!({
                "name": "test",
                "engine": "sqlite",
                "filePath": path.to_string_lossy(),
            }))
            .unwrap();
            Session::open(config, Secrets::default()).await.unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    async fn run(driver: &Arc<dyn Driver>, sql: &str) -> Vec<CursorEvent> {
        let mut cursor = driver.query(None, sql).await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = cursor.next(1000).await.unwrap() {
            events.push(event);
        }
        events
    }

    async fn count(driver: &Arc<dyn Driver>) -> String {
        match &run(driver, "SELECT count(*) FROM t").await[1] {
            CursorEvent::Rows(rows) => rows[0][0].to_display(),
            other => panic!("{other:?}"),
        }
    }

    /// A session with table `t` holding one row, and an open transaction that inserted a second.
    async fn session_with_transaction(db: &Scratch, state: &AppState) -> (String, Arc<dyn Driver>) {
        let session = db.open().await;
        run(&session.driver, "CREATE TABLE t (a INT)").await;
        run(&session.driver, "INSERT INTO t VALUES (0)").await;
        let session = state.insert_session("s".into(), session);
        let tx = state.txs.begin("s", &session.driver, None).await.unwrap();
        let driver = state.txs.get(&tx, Some("s")).unwrap().driver().clone();
        run(&driver, "INSERT INTO t VALUES (1)").await;
        assert_eq!(count(&driver).await, "2");
        (tx, driver)
    }

    #[tokio::test]
    async fn closing_a_session_rolls_back_its_transactions() {
        let db = Scratch::new();
        let state = AppState::default();
        let (tx, driver) = session_with_transaction(&db, &state).await;

        // A result of the transaction that is parked half-read holds its connection.
        let span = StatementSpan {
            start: 0,
            end: 0,
            sql: "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 50000) \
                  SELECT i FROM n"
                .into(),
            kind: StatementKind::Read,
            destructive: false,
        };
        let open: crate::runner::OpenFn = Arc::new(move |sql: String| {
            let driver = driver.clone();
            Box::pin(async move { driver.query(None, &sql).await })
        });
        let query = RunningQuery::start(
            "q".into(),
            "s".into(),
            open,
            &[span],
            RunOptions {
                tx_id: Some(tx.clone()),
                keep_tx: false,
                on_done: None,
            },
        );
        query.fetch(0, 0, 0, 10).await.unwrap();
        state.add_query(query);

        state.close_session("s").await;
        assert!(state.find_session("s").is_none());
        assert!(state.txs.get(&tx, None).is_err());

        // Nothing of the transaction reached the file, and its write lock is gone.
        let again = db.open().await;
        assert_eq!(count(&again.driver).await, "1");
        run(&again.driver, "INSERT INTO t VALUES (9)").await;
        again.close().await;
    }

    #[tokio::test]
    async fn closing_a_session_cancels_its_jobs_and_only_its_jobs() {
        let state = AppState::default();
        let mine = Job::for_session("s");
        let theirs = Job::for_session("other");
        let plain = Job::new();
        // A job that finishes once it is cancelled, like the dump import does.
        let report = mine.progress_fn();
        let token = mine.cancel.clone();
        crate::jobs::spawn(mine.clone(), async move {
            token.cancelled().await;
            let done = dbcore::model::JobProgress {
                rows: 0,
                bytes: 0,
                done: true,
                cancelled: true,
                error: None,
                warning: None,
            };
            report(&done);
            Ok(done)
        });
        state.add_job("mine".into(), mine.clone());
        state.add_job("theirs".into(), theirs.clone());
        state.add_job("plain".into(), plain.clone());

        let started = std::time::Instant::now();
        state.close_session("s").await;
        assert!(mine.snapshot().cancelled && mine.finished_at().is_some());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "it did not wait out the budget"
        );
        assert!(!theirs.cancel.is_cancelled());
        assert!(!plain.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn quitting_rolls_back_open_transactions() {
        let db = Scratch::new();
        let state = AppState::default();
        let (tx, _driver) = session_with_transaction(&db, &state).await;

        state.shutdown().await;
        assert!(state.txs.get(&tx, None).is_err());
        let again = db.open().await;
        assert_eq!(count(&again.driver).await, "1");
        run(&again.driver, "INSERT INTO t VALUES (9)").await;
        again.close().await;
    }
}
