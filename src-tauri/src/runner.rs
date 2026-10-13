//! The query runner: runs a script's statements one after another in a background
//! task and buffers their rows for the grid.
//!
//! Memory and latency are driven by the user's scrolling, not by the result size:
//!
//! * The first batch (up to 1000 rows) is read immediately, so the first row
//!   shows as soon as the server sends it.
//! * The reader of the LAST statement then parks until `fetch` asks for rows
//!   further down; it only reads while `loaded < highest requested end + 1000`.
//!   While it is parked the cursor's bounded channel applies backpressure to the
//!   server, so nothing else is buffered.
//! * Non-final statements are read up to 10,000 rows and then closed, because
//!   the next statement cannot start before they release their connection.
//!
//! Cancelling is cooperative (a `CancellationToken` selected against every await
//! in the reader) plus a server-side cancel through the cursor's `CancelHandle`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dbcore::driver::{CancelHandle, Cursor, CursorEvent};
use dbcore::export::CancellationToken;
use dbcore::model::{ColumnMeta, StatementSpan};
use dbcore::{Error, Row};
use tokio::sync::Notify;

use crate::commands::{QueryStatus, ResultSetStatus, RowsPage, StatementState, StatementStatus};

/// Rows requested from the cursor per call.
const BATCH: usize = 1000;
/// How far past the furthest requested row the reader of the last statement runs ahead.
const PREFETCH: u64 = 1000;
/// Rows kept of a statement that is followed by another one.
const NON_FINAL_CAP: usize = 10_000;
/// Longest `fetch` waits for rows that have not arrived yet.
const FETCH_WAIT: Duration = Duration::from_secs(5);
/// Longest the server-side cancel may take before we give up waiting for it.
const CANCEL_WAIT: Duration = Duration::from_secs(5);
/// Upper bound for one fetched page.
const MAX_PAGE: u64 = 50_000;

/// Opens one statement's cursor (the command binds the driver and database).
pub type OpenFn = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<Box<dyn Cursor>, Error>> + Send>>
        + Send
        + Sync,
>;

struct SetBuf {
    columns: Vec<ColumnMeta>,
    rows: Vec<Row>,
    complete: bool,
    truncated: bool,
}

struct Stmt {
    sql: String,
    state: StatementState,
    error: Option<Error>,
    affected: Option<u64>,
    sets: Vec<SetBuf>,
    started: Option<Instant>,
    elapsed_ms: u64,
    /// Present while the statement's cursor is alive.
    cancel: Option<Arc<dyn CancelHandle>>,
}

impl Stmt {
    fn elapsed(&self) -> u64 {
        match (self.state, self.started) {
            (StatementState::Running, Some(started)) => started.elapsed().as_millis() as u64,
            _ => self.elapsed_ms,
        }
    }

    /// Running -> Done, recording the time; anything else is left alone.
    fn finish(&mut self) {
        if self.state == StatementState::Running {
            self.state = StatementState::Done;
            self.elapsed_ms = self.started.map_or(0, |s| s.elapsed().as_millis() as u64);
        }
    }

    fn complete_sets(&mut self, truncated: bool) {
        for set in self.sets.iter_mut().filter(|s| !s.complete) {
            set.complete = true;
            set.truncated |= truncated;
        }
    }
}

struct Inner {
    stmts: Vec<Stmt>,
    /// Furthest row end (offset + limit) the UI asked for, per (statement, set).
    demand: HashMap<(usize, usize), u64>,
    /// Execution ended early (error or cancel): remaining statements stay pending.
    stopped: bool,
}

pub struct RunningQuery {
    pub id: String,
    pub session_id: String,
    inner: Mutex<Inner>,
    /// Rows, sets or states changed: wakes `fetch`.
    progress: Notify,
    /// Demand grew: wakes the parked reader.
    want_more: Notify,
    token: CancellationToken,
    created: Instant,
}

enum Flow {
    Next,
    Stop,
}

impl RunningQuery {
    /// Registers the statements and starts running them.
    pub fn start(
        id: String,
        session_id: String,
        open: OpenFn,
        spans: &[StatementSpan],
    ) -> Arc<RunningQuery> {
        let run = Arc::new(RunningQuery {
            id,
            session_id,
            inner: Mutex::new(Inner {
                stmts: spans
                    .iter()
                    .map(|s| Stmt {
                        sql: s.sql.clone(),
                        state: StatementState::Pending,
                        error: None,
                        affected: None,
                        sets: Vec::new(),
                        started: None,
                        elapsed_ms: 0,
                        cancel: None,
                    })
                    .collect(),
                demand: HashMap::new(),
                stopped: false,
            }),
            progress: Notify::new(),
            want_more: Notify::new(),
            token: CancellationToken::new(),
            created: Instant::now(),
        });

        let worker = tokio::spawn(run_all(run.clone(), open));
        let watcher = run.clone();
        tokio::spawn(async move {
            if let Err(e) = worker.await
                && e.is_panic()
            {
                watcher.fail_active(Error::Other(
                    "The query runner stopped unexpectedly.".into(),
                ));
            }
        });
        run
    }

    pub fn age(&self) -> Duration {
        self.created.elapsed()
    }

    pub fn is_finished(&self) -> bool {
        let inner = self.lock();
        Self::finished(&inner)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn finished(inner: &Inner) -> bool {
        inner.stopped
            || inner
                .stmts
                .iter()
                .all(|s| !matches!(s.state, StatementState::Pending | StatementState::Running))
    }

    // --- UI side -------------------------------------------------------------

    pub fn status(&self) -> QueryStatus {
        let inner = self.lock();
        QueryStatus {
            query_id: self.id.clone(),
            finished: Self::finished(&inner),
            statements: inner
                .stmts
                .iter()
                .enumerate()
                .map(|(index, s)| StatementStatus {
                    index,
                    sql: s.sql.clone(),
                    state: s.state,
                    error: s.error.clone(),
                    affected: s.affected,
                    sets: s
                        .sets
                        .iter()
                        .enumerate()
                        .map(|(set_index, set)| ResultSetStatus {
                            set_index,
                            columns: set.columns.clone(),
                            loaded: set.rows.len() as u64,
                            complete: set.complete,
                            truncated: set.truncated,
                        })
                        .collect(),
                    elapsed_ms: s.elapsed(),
                })
                .collect(),
        }
    }

    /// Rows `[offset, offset + limit)`; waits until they are loaded, the set is
    /// complete, or ~5 s passed.
    pub async fn fetch(
        &self,
        statement: usize,
        set: usize,
        offset: u64,
        limit: u32,
    ) -> Result<RowsPage, Error> {
        let limit = (limit as u64).min(MAX_PAGE);
        let end = offset.saturating_add(limit);
        self.request(statement, set, end);

        let deadline = tokio::time::Instant::now() + FETCH_WAIT;
        loop {
            // Register before looking, so a change between look and wait is not missed.
            let notified = self.progress.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            {
                let inner = self.lock();
                let stmt = inner
                    .stmts
                    .get(statement)
                    .ok_or_else(|| Error::NotFound(format!("statement {statement}")))?;
                match stmt.sets.get(set) {
                    Some(buf) => {
                        let loaded = buf.rows.len() as u64;
                        let settled = matches!(
                            stmt.state,
                            StatementState::Error | StatementState::Cancelled
                        );
                        if loaded >= end || buf.complete || settled {
                            let from = offset.min(loaded) as usize;
                            let to = end.min(loaded) as usize;
                            return Ok(RowsPage {
                                rows: buf.rows[from..to].to_vec(),
                                offset,
                                loaded,
                                complete: buf.complete,
                            });
                        }
                    }
                    None => {
                        let waiting = matches!(
                            stmt.state,
                            StatementState::Pending | StatementState::Running
                        ) && !inner.stopped;
                        if !waiting {
                            return match &stmt.error {
                                Some(e) => Err(e.clone()),
                                None => Err(Error::NotFound(format!(
                                    "result set {set} of statement {statement}"
                                ))),
                            };
                        }
                    }
                }
            }

            let timed_out = tokio::select! {
                _ = &mut notified => false,
                _ = tokio::time::sleep_until(deadline) => true,
            };
            if timed_out {
                let inner = self.lock();
                let loaded_buf = inner.stmts.get(statement).and_then(|s| s.sets.get(set));
                return Ok(match loaded_buf {
                    Some(buf) => {
                        let loaded = buf.rows.len() as u64;
                        let from = offset.min(loaded) as usize;
                        let to = end.min(loaded) as usize;
                        RowsPage {
                            rows: buf.rows[from..to].to_vec(),
                            offset,
                            loaded,
                            complete: buf.complete,
                        }
                    }
                    None => RowsPage {
                        rows: Vec::new(),
                        offset,
                        loaded: 0,
                        complete: false,
                    },
                });
            }
        }
    }

    /// Stops the running statement (on the server too) and everything after it.
    /// Rows already loaded stay available.
    pub async fn cancel(&self) {
        let handles: Vec<Arc<dyn CancelHandle>> = {
            let mut inner = self.lock();
            inner.stopped = true;
            let mut handles = Vec::new();
            for stmt in &mut inner.stmts {
                if stmt.state == StatementState::Running {
                    stmt.state = StatementState::Cancelled;
                    stmt.elapsed_ms = stmt.started.map_or(0, |s| s.elapsed().as_millis() as u64);
                }
                // Rows that were never read will not be: tell the grid it has everything there is.
                stmt.complete_sets(true);
                handles.extend(stmt.cancel.take());
            }
            handles
        };
        self.token.cancel();
        self.progress.notify_waiters();
        self.want_more.notify_waiters();
        for handle in handles {
            let _ = tokio::time::timeout(CANCEL_WAIT, handle.cancel()).await;
        }
    }

    /// Raises the furthest row the UI wants and wakes the reader if it is parked.
    fn request(&self, statement: usize, set: usize, end: u64) {
        let raised = {
            let mut inner = self.lock();
            let slot = inner.demand.entry((statement, set)).or_insert(0);
            let raised = end > *slot;
            *slot = (*slot).max(end);
            raised
        };
        if raised {
            self.want_more.notify_waiters();
        }
    }

    // --- reader side -----------------------------------------------------------

    fn start_statement(&self, idx: usize) -> bool {
        let mut inner = self.lock();
        if inner.stopped {
            return false;
        }
        let stmt = &mut inner.stmts[idx];
        stmt.state = StatementState::Running;
        stmt.started = Some(Instant::now());
        drop(inner);
        self.progress.notify_waiters();
        true
    }

    fn set_cancel_handle(&self, idx: usize, handle: Option<Arc<dyn CancelHandle>>) {
        let mut inner = self.lock();
        if handle.is_some() && inner.stopped {
            // Cancelled while the cursor was opening: the reader sees the token at its next await.
            return;
        }
        inner.stmts[idx].cancel = handle;
    }

    fn fail(&self, idx: usize, error: Error) {
        {
            let mut inner = self.lock();
            let stmt = &mut inner.stmts[idx];
            if stmt.state == StatementState::Cancelled {
                return;
            }
            let error = error.with_statement(&stmt.sql);
            stmt.state = StatementState::Error;
            stmt.error = Some(error);
            stmt.elapsed_ms = stmt.started.map_or(0, |s| s.elapsed().as_millis() as u64);
            stmt.cancel = None;
            stmt.complete_sets(false);
            inner.stopped = true;
        }
        self.progress.notify_waiters();
    }

    /// Marks the statement that is running (or the last one started) as failed.
    fn fail_active(&self, error: Error) {
        let idx = {
            let inner = self.lock();
            inner
                .stmts
                .iter()
                .position(|s| s.state == StatementState::Running)
                .or_else(|| {
                    inner
                        .stmts
                        .iter()
                        .rposition(|s| s.state != StatementState::Pending)
                })
        };
        if let Some(idx) = idx {
            self.fail(idx, error);
        }
    }

    fn begin_set(&self, idx: usize, columns: Vec<ColumnMeta>) -> usize {
        let index = {
            let mut inner = self.lock();
            let stmt = &mut inner.stmts[idx];
            stmt.complete_sets(false);
            stmt.sets.push(SetBuf {
                columns,
                rows: Vec::new(),
                complete: false,
                truncated: false,
            });
            stmt.sets.len() - 1
        };
        self.progress.notify_waiters();
        index
    }

    /// Appends a batch. Returns true when reading of this statement should stop
    /// (the cap of a non-final statement was hit).
    fn append_rows(&self, idx: usize, set: usize, rows: Vec<Row>, is_last: bool) -> bool {
        let stop = {
            let mut inner = self.lock();
            let stmt = &mut inner.stmts[idx];
            let buf = &mut stmt.sets[set];
            buf.rows.extend(rows);
            let mut stop = false;
            if !is_last && buf.rows.len() > NON_FINAL_CAP {
                buf.rows.truncate(NON_FINAL_CAP);
                buf.complete = true;
                buf.truncated = true;
                stop = true;
            }
            if is_last || stop {
                // For the last statement the first rows are the "result": from here
                // on the UI can show it while the rest streams in on demand.
                stmt.finish();
            }
            if stop {
                stmt.cancel = None;
            }
            stop
        };
        self.progress.notify_waiters();
        stop
    }

    fn add_affected(&self, idx: usize, n: u64) {
        let mut inner = self.lock();
        let stmt = &mut inner.stmts[idx];
        stmt.affected = Some(stmt.affected.unwrap_or(0).saturating_add(n));
    }

    /// The cursor reported its end: everything is read.
    fn finish_stream(&self, idx: usize) {
        {
            let mut inner = self.lock();
            let stmt = &mut inner.stmts[idx];
            stmt.complete_sets(false);
            stmt.finish();
            stmt.cancel = None;
        }
        self.progress.notify_waiters();
    }

    fn loaded(&self, idx: usize, set: usize) -> usize {
        self.lock().stmts[idx].sets[set].rows.len()
    }

    /// Parks until the UI wants rows beyond what is loaded. False when cancelled.
    async fn wait_for_demand(&self, idx: usize, set: usize) -> bool {
        loop {
            let notified = self.want_more.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let inner = self.lock();
                let loaded = inner.stmts[idx].sets[set].rows.len() as u64;
                let demand = inner.demand.get(&(idx, set)).copied().unwrap_or(0);
                if loaded < demand + PREFETCH {
                    return true;
                }
            }
            tokio::select! {
                biased;
                _ = self.token.cancelled() => return false,
                _ = &mut notified => {}
            }
        }
    }
}

async fn run_all(run: Arc<RunningQuery>, open: OpenFn) {
    let count = run.lock().stmts.len();
    for idx in 0..count {
        let is_last = idx + 1 == count;
        if !run.start_statement(idx) {
            return;
        }
        let sql = run.lock().stmts[idx].sql.clone();

        let opened = tokio::select! {
            biased;
            _ = run.token.cancelled() => return,
            opened = open(sql) => opened,
        };
        let mut cursor = match opened {
            Ok(cursor) => cursor,
            Err(e) => {
                run.fail(idx, e);
                return;
            }
        };
        run.set_cancel_handle(idx, Some(cursor.cancel_handle()));
        let flow = read_statement(&run, idx, is_last, &mut *cursor).await;
        // Dropping the cursor releases the connection (and stops a streaming statement).
        drop(cursor);
        match flow {
            Ok(Flow::Next) => {}
            Ok(Flow::Stop) => return,
            Err(e) => {
                run.fail(idx, e);
                return;
            }
        }
    }
}

async fn read_statement(
    run: &RunningQuery,
    idx: usize,
    is_last: bool,
    cursor: &mut dyn Cursor,
) -> Result<Flow, Error> {
    let mut current: Option<usize> = None;
    loop {
        let max_rows = match current {
            Some(set) if is_last => {
                if !run.wait_for_demand(idx, set).await {
                    return Ok(Flow::Stop);
                }
                BATCH
            }
            Some(set) => (NON_FINAL_CAP + 1)
                .saturating_sub(run.loaded(idx, set))
                .clamp(1, BATCH),
            None => BATCH,
        };

        let event = tokio::select! {
            biased;
            _ = run.token.cancelled() => return Ok(Flow::Stop),
            event = cursor.next(max_rows) => event?,
        };
        match event {
            None => {
                run.finish_stream(idx);
                return Ok(Flow::Next);
            }
            Some(CursorEvent::Columns(columns)) => current = Some(run.begin_set(idx, columns)),
            Some(CursorEvent::Rows(rows)) => {
                let set = match current {
                    Some(set) => set,
                    None => *current.insert(run.begin_set(idx, Vec::new())),
                };
                if run.append_rows(idx, set, rows, is_last) {
                    return Ok(Flow::Next);
                }
            }
            Some(CursorEvent::Affected(n)) => run.add_affected(idx, n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbcore::Value;
    use dbcore::cursor::VecCursor;
    use dbcore::model::{StatementKind, TypeFamily};

    /// `"n"` yields n rows, `"boom"` fails.
    fn fake() -> OpenFn {
        Arc::new(|sql: String| {
            Box::pin(async move {
                if sql == "boom" {
                    return Err(Error::database("boom"));
                }
                let n: i64 = sql.parse().unwrap_or(0);
                let column = ColumnMeta {
                    name: "n".into(),
                    type_name: "int".into(),
                    family: TypeFamily::Integer,
                };
                let rows: Vec<Row> = (0..n).map(|i| vec![Value::Int(i)]).collect();
                Ok(Box::new(VecCursor::new(vec![
                    CursorEvent::Columns(vec![column]),
                    CursorEvent::Rows(rows),
                ])) as Box<dyn Cursor>)
            })
        })
    }

    fn start(sqls: &[&str]) -> Arc<RunningQuery> {
        let spans: Vec<StatementSpan> = sqls
            .iter()
            .map(|s| StatementSpan {
                start: 0,
                end: s.len(),
                sql: s.to_string(),
                kind: StatementKind::Read,
                destructive: false,
            })
            .collect();
        RunningQuery::start("q".into(), "s".into(), fake(), &spans)
    }

    #[tokio::test]
    async fn last_statement_reads_on_demand() {
        let run = start(&["5000"]);
        let page = run.fetch(0, 0, 0, 100).await.unwrap();
        assert_eq!(page.rows.len(), 100);
        // Nothing past the prefetch window is read until the UI scrolls there.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let status = run.status();
        let set = &status.statements[0].sets[0];
        assert!(
            set.loaded <= 100 + PREFETCH + BATCH as u64,
            "loaded {}",
            set.loaded
        );
        assert!(!set.complete);
        assert_eq!(status.statements[0].state, StatementState::Done);

        let tail = run.fetch(0, 0, 4900, 100).await.unwrap();
        assert_eq!(tail.rows.len(), 100);
        assert_eq!(tail.rows[99][0], Value::Int(4999));
        let end = run.fetch(0, 0, 5000, 100).await.unwrap();
        assert!(end.rows.is_empty() && end.complete && end.loaded == 5000);
    }

    #[tokio::test]
    async fn non_final_statements_are_capped_and_the_next_one_runs() {
        let run = start(&["25000", "3"]);
        let page = run.fetch(1, 0, 0, 10).await.unwrap();
        assert_eq!(page.rows.len(), 3);
        let status = run.status();
        let first = &status.statements[0].sets[0];
        assert_eq!(first.loaded, NON_FINAL_CAP as u64);
        assert!(first.complete && first.truncated);
        assert!(status.finished);
    }

    #[tokio::test]
    async fn stops_at_the_first_error() {
        let run = start(&["1", "boom", "2"]);
        let err = run
            .fetch(1, 0, 0, 10)
            .await
            .err()
            .expect("statement 1 failed");
        assert!(err.to_string().contains("boom"));
        let status = run.status();
        assert_eq!(status.statements[0].state, StatementState::Done);
        assert_eq!(status.statements[1].state, StatementState::Error);
        assert_eq!(status.statements[2].state, StatementState::Pending);
        assert!(status.finished);
    }

    #[tokio::test]
    async fn cancel_freezes_what_was_loaded() {
        let run = start(&["5000"]);
        run.fetch(0, 0, 0, 10).await.unwrap();
        run.cancel().await;
        let status = run.status();
        assert!(status.finished);
        let set = &status.statements[0].sets[0];
        assert!(set.complete && set.truncated);
        // Fetching after a cancel returns immediately instead of waiting for more.
        let started = Instant::now();
        run.fetch(0, 0, 4000, 10).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
