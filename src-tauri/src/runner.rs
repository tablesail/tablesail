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
//!
//! A query can run on a connection that belongs to a manual transaction (`RunOptions::tx_id`)
//! or on one opened for the script alone (`RunOptions::on_done` closes it). Closing the
//! result of such a query must not end the transaction: statements that already delivered
//! their first rows are left to finish in the background instead of being cancelled when the
//! engine would abort the transaction for it (`RunOptions::keep_tx`).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Runs when the query is over and all its cursors are gone (closes a connection opened for it).
pub type DoneFn = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// Where and how a query runs, beyond its statements.
#[derive(Default)]
pub struct RunOptions {
    /// The manual transaction the statements run in.
    pub tx_id: Option<String>,
    /// Cancelling the query must leave the transaction alone: no server-side cancel for a
    /// statement that has already delivered its rows (Postgres aborts a transaction for it).
    pub keep_tx: bool,
    pub on_done: Option<DoneFn>,
}

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
    pub tx_id: Option<String>,
    keep_tx: bool,
    inner: Mutex<Inner>,
    /// Rows, sets or states changed: wakes `fetch`.
    progress: Notify,
    /// Demand grew: wakes the parked reader.
    want_more: Notify,
    token: CancellationToken,
    created: Instant,
    on_done: Mutex<Option<DoneFn>>,
    /// The runner has stopped and `on_done` has run.
    ended: AtomicBool,
    ended_notify: Notify,
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
        options: RunOptions,
    ) -> Arc<RunningQuery> {
        let run = Arc::new(RunningQuery {
            id,
            session_id,
            tx_id: options.tx_id,
            keep_tx: options.keep_tx,
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
            on_done: Mutex::new(options.on_done),
            ended: AtomicBool::new(false),
            ended_notify: Notify::new(),
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
            watcher.end().await;
        });
        run
    }

    /// The runner is done with its cursors: runs `on_done` and wakes `wait_ended`.
    async fn end(&self) {
        let done = self
            .on_done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(done) = done {
            done().await;
        }
        self.ended.store(true, Ordering::SeqCst);
        self.ended_notify.notify_waiters();
    }

    /// Waits (at most `limit`) until the runner has stopped and cleaned up after itself.
    async fn wait_ended(&self, limit: Duration) {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let notified = self.ended_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.ended.load(Ordering::SeqCst) {
                return;
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = tokio::time::sleep_until(deadline) => return,
            }
        }
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
        self.stop(self.keep_tx).await;
    }

    /// Like `cancel`, for when the connection is about to be rolled back or closed: every
    /// statement is cancelled on the server so nothing holds the connection up, and the call
    /// returns once the runner has let go of it.
    pub async fn abort(&self) {
        self.stop(false).await;
        self.wait_ended(CANCEL_WAIT).await;
    }

    /// Stops reading, so the query lets go of its connection, unless a statement is still
    /// running (then nothing is touched). Rows already loaded stay available.
    pub async fn settle(&self) {
        let running = self
            .lock()
            .stmts
            .iter()
            .any(|s| s.state == StatementState::Running);
        if !running {
            self.stop(true).await;
        }
    }

    /// `spare_settled`: statements that already delivered their rows are not cancelled on the
    /// server (only the reading stops).
    async fn stop(&self, spare_settled: bool) {
        let handles: Vec<Arc<dyn CancelHandle>> = {
            let mut inner = self.lock();
            inner.stopped = true;
            let mut handles = Vec::new();
            for stmt in &mut inner.stmts {
                let running = stmt.state == StatementState::Running;
                if running {
                    stmt.state = StatementState::Cancelled;
                    stmt.elapsed_ms = stmt.started.map_or(0, |s| s.elapsed().as_millis() as u64);
                }
                // Rows that were never read will not be: tell the grid it has everything there is.
                stmt.complete_sets(true);
                if running || !spare_settled {
                    handles.extend(stmt.cancel.take());
                }
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
    use async_trait::async_trait;
    use dbcore::Value;
    use dbcore::cursor::VecCursor;
    use dbcore::model::{StatementKind, TypeFamily};
    use std::sync::atomic::AtomicUsize;

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
        RunningQuery::start(
            "q".into(),
            "s".into(),
            fake(),
            &spans,
            RunOptions::default(),
        )
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

    // --- transactions and dedicated connections ---------------------------------

    struct CountingCancel(Arc<AtomicUsize>);

    #[async_trait]
    impl CancelHandle for CountingCancel {
        async fn cancel(&self) -> Result<(), Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A cursor that counts server-side cancels; `hang` never produces anything.
    struct Counted {
        inner: VecCursor,
        cancels: Arc<AtomicUsize>,
        hang: bool,
    }

    #[async_trait]
    impl Cursor for Counted {
        async fn next(&mut self, max_rows: usize) -> Result<Option<CursorEvent>, Error> {
            if self.hang {
                std::future::pending::<()>().await;
            }
            self.inner.next(max_rows).await
        }

        fn cancel_handle(&self) -> Arc<dyn CancelHandle> {
            Arc::new(CountingCancel(self.cancels.clone()))
        }
    }

    /// `"n"` yields n rows, `"hang"` never answers; every cursor counts its cancels in `cancels`.
    fn counting(cancels: &Arc<AtomicUsize>) -> OpenFn {
        let cancels = cancels.clone();
        Arc::new(move |sql: String| {
            let cancels = cancels.clone();
            Box::pin(async move {
                let n: i64 = sql.parse().unwrap_or(0);
                let column = ColumnMeta {
                    name: "n".into(),
                    type_name: "int".into(),
                    family: TypeFamily::Integer,
                };
                let rows: Vec<Row> = (0..n).map(|i| vec![Value::Int(i)]).collect();
                Ok(Box::new(Counted {
                    inner: VecCursor::new(vec![
                        CursorEvent::Columns(vec![column]),
                        CursorEvent::Rows(rows),
                    ]),
                    cancels,
                    hang: sql == "hang",
                }) as Box<dyn Cursor>)
            })
        })
    }

    fn start_counting(
        sqls: &[&str],
        cancels: &Arc<AtomicUsize>,
        options: RunOptions,
    ) -> Arc<RunningQuery> {
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
        RunningQuery::start("q".into(), "s".into(), counting(cancels), &spans, options)
    }

    fn cancelled(cancels: &Arc<AtomicUsize>) -> usize {
        cancels.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn closing_a_result_in_a_transaction_does_not_cancel_it_on_the_server() {
        // Postgres aborts the whole transaction for a cancelled statement, so a result that
        // is merely being scrolled is only left alone, not cancelled.
        let cancels = Arc::new(AtomicUsize::new(0));
        let keep = RunOptions {
            tx_id: Some("tx".into()),
            keep_tx: true,
            on_done: None,
        };
        let run = start_counting(&["5000"], &cancels, keep);
        run.fetch(0, 0, 0, 10).await.unwrap();
        run.cancel().await;
        run.wait_ended(Duration::from_secs(2)).await;
        assert_eq!(cancelled(&cancels), 0);
        assert!(run.status().finished);

        // Elsewhere closing it cancels the server side, as before.
        let run = start_counting(&["5000"], &cancels, RunOptions::default());
        run.fetch(0, 0, 0, 10).await.unwrap();
        run.cancel().await;
        assert_eq!(cancelled(&cancels), 1);
    }

    #[tokio::test]
    async fn a_running_statement_is_cancelled_even_in_a_transaction() {
        let cancels = Arc::new(AtomicUsize::new(0));
        let keep = RunOptions {
            tx_id: Some("tx".into()),
            keep_tx: true,
            on_done: None,
        };
        let run = start_counting(&["hang"], &cancels, keep);
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Settling leaves a statement that is still working alone.
        run.settle().await;
        assert_eq!(cancelled(&cancels), 0);
        assert!(!run.status().finished);
        // The user's own cancel stops it.
        run.cancel().await;
        assert_eq!(cancelled(&cancels), 1);
        assert_eq!(run.status().statements[0].state, StatementState::Cancelled);
    }

    #[tokio::test]
    async fn settling_frees_a_parked_result_and_keeps_its_rows() {
        let cancels = Arc::new(AtomicUsize::new(0));
        let keep = RunOptions {
            tx_id: Some("tx".into()),
            keep_tx: true,
            on_done: None,
        };
        let run = start_counting(&["5000"], &cancels, keep);
        run.fetch(0, 0, 0, 10).await.unwrap();
        run.settle().await;
        run.wait_ended(Duration::from_secs(2)).await;
        assert!(
            run.ended.load(Ordering::SeqCst),
            "the reader should let go of its cursor"
        );
        assert_eq!(cancelled(&cancels), 0);
        let page = run.fetch(0, 0, 0, 10).await.unwrap();
        assert_eq!(page.rows.len(), 10);
    }

    #[tokio::test]
    async fn abort_cancels_everything_and_waits_for_the_cleanup() {
        let cancels = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        let options = RunOptions {
            tx_id: Some("tx".into()),
            keep_tx: true,
            on_done: Some(Box::new({
                let cleaned = cleaned.clone();
                move || {
                    Box::pin(async move {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        cleaned.fetch_add(1, Ordering::SeqCst);
                    })
                }
            })),
        };
        let run = start_counting(&["5000"], &cancels, options);
        run.fetch(0, 0, 0, 10).await.unwrap();
        run.abort().await;
        // Even a statement that had delivered its rows is cancelled, and the cleanup has run.
        assert_eq!(cancelled(&cancels), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_cleanup_runs_once_when_the_script_ends_by_itself() {
        let cancels = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        let options = RunOptions {
            tx_id: None,
            keep_tx: false,
            on_done: Some(Box::new({
                let cleaned = cleaned.clone();
                move || {
                    Box::pin(async move {
                        cleaned.fetch_add(1, Ordering::SeqCst);
                    })
                }
            })),
        };
        let run = start_counting(&["1", "2", "3"], &cancels, options);
        run.wait_ended(Duration::from_secs(2)).await;
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
        assert!(run.status().finished);
        // Closing afterwards does not run it again.
        run.abort().await;
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }
}
