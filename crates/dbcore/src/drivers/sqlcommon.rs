//! Helpers shared by the SQL drivers (Postgres, MySQL/MariaDB, SQLite).
//!
//! - building browse/count/insert SQL from the model types
//! - the structure cache
//! - `Pump`: turns a driver's row stream into batched `CursorEvent`s with backpressure
//! - `IdleList`/`Lease`: sticky dedicated connections for `query()` (so `BEGIN` and
//!   `SET` survive between statements of a script) that are cancelled and discarded
//!   when a cursor is dropped mid-query
//! - generic transaction runners for change sets and imports

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;

use crate::cursor::{EventSender, PRODUCER_BATCH, send};
use crate::dialect::Dialect;
use crate::driver::CursorEvent;
use crate::error::{Error, Result};
use crate::model::{
    ApplyResult, BrowseRequest, ColumnMeta, Editability, RowChange, TableKind, TableRef,
    TableStructure,
};
use crate::value::Row;

/// How long a fetched `TableStructure` is reused (preview, then apply, reuse one fetch).
pub const STRUCTURE_TTL: Duration = Duration::from_secs(15);
/// How long a primary key lookup is reused for page-after-page browsing.
pub const PK_TTL: Duration = Duration::from_secs(120);
/// Unfiltered tables estimated above this many rows are not counted exactly.
pub const ESTIMATE_ONLY_ABOVE: u64 = 1_000_000;
/// Rows per multi-row INSERT when importing.
pub const INSERT_BATCH_ROWS: usize = 500;
/// Upper bound on one INSERT statement's text, to stay below `max_allowed_packet` (4 MB on old MySQL).
pub const INSERT_BATCH_BYTES: usize = 1024 * 1024;
/// Rows already on the wire are flushed to the consumer at least this often.
const FLUSH_INTERVAL: Duration = Duration::from_millis(25);
/// A pooled connection idle for longer than this is pinged before reuse.
const IDLE_PING_AFTER: Duration = Duration::from_secs(30);

pub const NO_PK_REASON: &str = "This table has no primary key, so rows can't be identified safely.";

// ---------------------------------------------------------------------------
// Structure cache
// ---------------------------------------------------------------------------

type Timed<T> = HashMap<TableRef, (Instant, Arc<T>)>;

#[derive(Default)]
pub struct StructureCache {
    structures: Mutex<Timed<TableStructure>>,
    keys: Mutex<Timed<Vec<String>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl StructureCache {
    pub fn structure(&self, table: &TableRef) -> Option<Arc<TableStructure>> {
        let map = lock(&self.structures);
        map.get(table)
            .filter(|(at, _)| at.elapsed() < STRUCTURE_TTL)
            .map(|(_, s)| s.clone())
    }

    pub fn put_structure(&self, structure: TableStructure) -> Arc<TableStructure> {
        let s = Arc::new(structure);
        let mut map = lock(&self.structures);
        map.retain(|_, (at, _)| at.elapsed() < STRUCTURE_TTL);
        map.insert(s.table.clone(), (Instant::now(), s.clone()));
        drop(map);
        lock(&self.keys).insert(
            s.table.clone(),
            (Instant::now(), Arc::new(s.primary_key.clone())),
        );
        s
    }

    /// Primary key columns, from a cached structure or an earlier key lookup.
    pub fn primary_key(&self, table: &TableRef) -> Option<Arc<Vec<String>>> {
        if let Some(s) = self.structure(table) {
            return Some(Arc::new(s.primary_key.clone()));
        }
        lock(&self.keys)
            .get(table)
            .filter(|(at, _)| at.elapsed() < PK_TTL)
            .map(|(_, k)| k.clone())
    }

    pub fn put_primary_key(&self, table: &TableRef, key: Vec<String>) -> Arc<Vec<String>> {
        let key = Arc::new(key);
        lock(&self.keys).insert(table.clone(), (Instant::now(), key.clone()));
        key
    }

    pub fn clear(&self) {
        lock(&self.structures).clear();
        lock(&self.keys).clear();
    }
}

/// Why a table of this kind and key can't be edited, as a sentence for the UI.
pub fn editability(kind: TableKind, primary_key: &[String]) -> Editability {
    let reason = match kind {
        TableKind::View => Some("Views can't be edited."),
        TableKind::MaterializedView => Some("Materialized views can't be edited."),
        TableKind::Virtual => Some("Virtual tables can't be edited."),
        TableKind::Sequence => Some("Sequences can't be edited."),
        TableKind::System => Some("System tables can't be edited."),
        TableKind::Table
        | TableKind::ForeignTable
        | TableKind::Collection
        | TableKind::TimeSeries => None,
    };
    match reason {
        Some(r) => Editability::ReadOnly(r.to_owned()),
        None if primary_key.is_empty() => Editability::ReadOnly(NO_PK_REASON.to_owned()),
        None => Editability::Editable,
    }
}

// ---------------------------------------------------------------------------
// SQL building
// ---------------------------------------------------------------------------

/// True when the request has neither filters nor a raw WHERE clause.
pub fn is_unfiltered(req: &BrowseRequest) -> bool {
    req.filters.is_empty() && raw_where(req).is_none()
}

/// The raw WHERE text, trimmed and without a leading `WHERE` keyword.
fn raw_where(req: &BrowseRequest) -> Option<&str> {
    let raw = req.raw_where.as_deref()?.trim();
    let raw = match raw.get(..5) {
        Some(head)
            if head.eq_ignore_ascii_case("where")
                && raw[5..].starts_with(|c: char| c.is_whitespace() || c == '(') =>
        {
            raw[5..].trim_start()
        }
        _ => raw,
    };
    (!raw.is_empty()).then_some(raw)
}

/// Filters combined with AND/OR, ANDed with the raw WHERE clause.
pub fn where_sql(d: Dialect, req: &BrowseRequest) -> Result<Option<String>> {
    let mut parts = Vec::new();
    if !req.filters.is_empty() {
        let exprs = req
            .filters
            .iter()
            .map(|f| d.filter_sql(f))
            .collect::<Result<Vec<_>>>()?;
        let joined = exprs.join(if req.match_all { " AND " } else { " OR " });
        parts.push(if exprs.len() > 1 {
            format!("({joined})")
        } else {
            joined
        });
    }
    if let Some(raw) = raw_where(req) {
        // Newlines keep a trailing `-- comment` from swallowing the closing parenthesis.
        parts.push(format!("(\n{raw}\n)"));
    }
    Ok((!parts.is_empty()).then(|| parts.join(" AND ")))
}

/// `SELECT cols FROM table [WHERE …] [ORDER BY …] [LIMIT n OFFSET m]`.
/// Without an explicit sort, `default_order` (the primary key) orders the rows.
pub fn select_sql(
    d: Dialect,
    req: &BrowseRequest,
    default_order: &[String],
    page: Option<(u32, u64)>,
) -> Result<String> {
    let cols = if req.columns.is_empty() {
        "*".to_owned()
    } else {
        req.columns
            .iter()
            .map(|c| d.quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut sql = format!("SELECT {cols} FROM {}", d.qualified(&req.table));
    if let Some(w) = where_sql(d, req)? {
        sql.push_str(" WHERE ");
        sql.push_str(&w);
    }
    let order: Vec<String> = if req.sort.is_empty() {
        default_order.iter().map(|c| d.quote_ident(c)).collect()
    } else {
        req.sort
            .iter()
            .map(|s| {
                format!(
                    "{} {}",
                    d.quote_ident(&s.column),
                    if s.descending { "DESC" } else { "ASC" }
                )
            })
            .collect()
    };
    if !order.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(&order.join(", "));
    }
    if let Some((limit, offset)) = page {
        sql.push(' ');
        sql.push_str(&d.limit_offset(limit, offset));
    }
    Ok(sql)
}

pub fn count_sql(d: Dialect, req: &BrowseRequest) -> Result<String> {
    let mut sql = format!("SELECT COUNT(*) FROM {}", d.qualified(&req.table));
    if let Some(w) = where_sql(d, req)? {
        sql.push_str(" WHERE ");
        sql.push_str(&w);
    }
    Ok(sql)
}

/// Multi-row INSERT statements of text literals (`None` → NULL), split by row count and size.
pub fn insert_statements(
    d: Dialect,
    table: &TableRef,
    columns: &[String],
    rows: &[Vec<Option<String>>],
) -> Result<Vec<String>> {
    if columns.is_empty() {
        return Err(Error::InvalidInput(
            "Choose at least one column to import into.".to_owned(),
        ));
    }
    let head = format!(
        "INSERT INTO {} ({}) VALUES ",
        d.qualified(table),
        columns
            .iter()
            .map(|c| d.quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_cur = 0;
    for (n, row) in rows.iter().enumerate() {
        if row.len() != columns.len() {
            return Err(Error::InvalidInput(format!(
                "Row {} has {} values but {} columns were mapped.",
                n + 1,
                row.len(),
                columns.len()
            )));
        }
        let tuple = format!(
            "({})",
            row.iter()
                .map(|v| match v {
                    Some(s) => d.quote_string(s),
                    None => "NULL".to_owned(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        );
        if in_cur > 0
            && (in_cur >= INSERT_BATCH_ROWS || cur.len() + tuple.len() > INSERT_BATCH_BYTES)
        {
            out.push(std::mem::take(&mut cur));
            in_cur = 0;
        }
        if in_cur == 0 {
            cur.push_str(&head);
        } else {
            cur.push_str(", ");
        }
        cur.push_str(&tuple);
        in_cur += 1;
    }
    if in_cur > 0 {
        out.push(cur);
    }
    Ok(out)
}

pub fn meta(
    name: impl Into<String>,
    type_name: impl Into<String>,
    family: crate::model::TypeFamily,
) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: type_name.into(),
        family,
    }
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// Reads the affected-row count of a driver's `QueryResult`.
pub trait RowsAffected {
    fn affected(&self) -> u64;
}

impl RowsAffected for sqlx::postgres::PgQueryResult {
    fn affected(&self) -> u64 {
        self.rows_affected()
    }
}

impl RowsAffected for sqlx::mysql::MySqlQueryResult {
    fn affected(&self) -> u64 {
        self.rows_affected()
    }
}

impl RowsAffected for sqlx::sqlite::SqliteQueryResult {
    fn affected(&self) -> u64 {
        self.rows_affected()
    }
}

/// Runs `statements` in one transaction. For each UPDATE/DELETE in `changes`
/// (statement i belongs to change i) zero affected rows aborts and rolls back.
pub async fn apply_in_transaction<DB>(
    pool: &sqlx::Pool<DB>,
    statements: &[String],
    changes: &[RowChange],
) -> Result<ApplyResult>
where
    DB: sqlx::Database,
    DB::QueryResult: RowsAffected,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
{
    let mut tx = pool.begin().await?;
    let mut total = 0;
    for (i, sql) in statements.iter().enumerate() {
        let outcome = sqlx::raw_sql(sqlx::AssertSqlSafe(sql.clone()))
            .execute(&mut *tx)
            .await;
        let failure = match outcome {
            Ok(res) => {
                let n = res.affected();
                let must_match = matches!(
                    changes.get(i),
                    Some(RowChange::Update { .. } | RowChange::Delete { .. })
                );
                if must_match && n == 0 {
                    Some(Error::database(
                        "Row was changed or deleted by someone else",
                    ))
                } else {
                    total += n;
                    None
                }
            }
            Err(e) => Some(Error::from(e)),
        };
        if let Some(err) = failure {
            let _ = tx.rollback().await;
            return Err(err.with_statement(sql));
        }
    }
    tx.commit().await?;
    Ok(ApplyResult {
        statements: statements.len() as u64,
        rows_affected: total,
    })
}

/// Runs INSERT batches in one transaction; returns the number of rows inserted.
pub async fn insert_in_transaction<DB>(pool: &sqlx::Pool<DB>, statements: &[String]) -> Result<u64>
where
    DB: sqlx::Database,
    DB::QueryResult: RowsAffected,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
{
    let mut tx = pool.begin().await?;
    let mut total = 0;
    for sql in statements {
        match sqlx::raw_sql(sqlx::AssertSqlSafe(sql.clone()))
            .execute(&mut *tx)
            .await
        {
            Ok(res) => total += res.affected(),
            Err(e) => {
                let _ = tx.rollback().await;
                return Err(Error::from(e));
            }
        }
    }
    tx.commit().await?;
    Ok(total)
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

/// Shared between a running statement and its cancel handle.
#[derive(Default)]
pub struct CancelState {
    /// A statement is running on the connection (so a cancel request has a target).
    pub active: AtomicBool,
    /// Cancellation was requested; the error the server then returns means "cancelled".
    pub requested: AtomicBool,
}

impl CancelState {
    pub fn start(&self) {
        self.requested.store(false, Ordering::SeqCst);
        self.active.store(true, Ordering::SeqCst);
    }

    pub fn finish(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    /// Marks a cancel request; false when nothing is running.
    pub fn request(&self) -> bool {
        if self.active.load(Ordering::SeqCst) {
            self.requested.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Server errors after a cancel request mean the cancel took effect.
    pub fn map_error(&self, err: Error) -> Error {
        if self.requested.load(Ordering::SeqCst) && !matches!(err, Error::Cancelled) {
            Error::Cancelled
        } else {
            err
        }
    }
}

/// Groups rows into `CursorEvent`s for the bounded channel. The first row of a
/// result set is sent alone so it reaches the grid without waiting for a batch.
pub struct Pump {
    tx: EventSender,
    batch: Vec<Row>,
    in_set: bool,
    first_row_of_set: bool,
    last_flush: Instant,
    grid: bool,
}

impl Pump {
    pub fn new(tx: EventSender, grid: bool) -> Self {
        Pump {
            tx,
            batch: Vec::new(),
            in_set: false,
            first_row_of_set: false,
            last_flush: Instant::now(),
            grid,
        }
    }

    /// Whether values should be truncated for the grid.
    pub fn grid(&self) -> bool {
        self.grid
    }

    /// True between `columns` and `end_set`.
    pub fn in_set(&self) -> bool {
        self.in_set
    }

    pub async fn columns(&mut self, columns: Vec<ColumnMeta>) -> Result<()> {
        self.flush().await?;
        send(&self.tx, CursorEvent::Columns(columns)).await?;
        self.in_set = true;
        self.first_row_of_set = true;
        Ok(())
    }

    pub async fn row(&mut self, row: Row) -> Result<()> {
        if self.batch.capacity() == 0 {
            self.batch.reserve(PRODUCER_BATCH);
        }
        self.batch.push(row);
        if self.first_row_of_set
            || self.batch.len() >= PRODUCER_BATCH
            || self.last_flush.elapsed() >= FLUSH_INTERVAL
        {
            self.first_row_of_set = false;
            self.flush().await?;
        }
        Ok(())
    }

    pub async fn flush(&mut self) -> Result<()> {
        if !self.batch.is_empty() {
            let rows = std::mem::take(&mut self.batch);
            send(&self.tx, CursorEvent::Rows(rows)).await?;
        }
        self.last_flush = Instant::now();
        Ok(())
    }

    /// Ends the current result set. A statement that never produced columns
    /// reports `Affected(n)`, unless `defer` is set (a SELECT that returned no
    /// rows: the caller describes it and emits `Columns` instead); then the count is returned.
    pub async fn end_set(&mut self, affected: u64, defer: bool) -> Result<Option<u64>> {
        self.flush().await?;
        if self.in_set {
            self.in_set = false;
            return Ok(None);
        }
        if defer {
            return Ok(Some(affected));
        }
        send(&self.tx, CursorEvent::Affected(affected)).await?;
        Ok(None)
    }

    pub async fn affected(&mut self, affected: u64) -> Result<()> {
        self.flush().await?;
        send(&self.tx, CursorEvent::Affected(affected)).await
    }
}

// ---------------------------------------------------------------------------
// Dedicated connections
// ---------------------------------------------------------------------------

/// A connection parked between statements, with what the driver learned about it.
pub struct Idle<C> {
    pub conn: C,
    /// Backend pid (Postgres), connection id (MySQL), handle address (SQLite).
    pub id: u64,
    /// Current database (MySQL `USE`), when tracked.
    pub db: Option<String>,
    pub since: Instant,
}

impl<C> Idle<C> {
    /// Idle long enough that it should be checked before reuse.
    pub fn is_stale(&self) -> bool {
        self.since.elapsed() > IDLE_PING_AFTER
    }
}

/// LIFO list of parked connections: consecutive statements of a script land on the
/// same connection, so transactions and session settings carry over.
pub struct IdleList<C> {
    items: Mutex<Vec<Idle<C>>>,
    max: usize,
}

impl<C> IdleList<C> {
    pub fn new(max: usize) -> Self {
        IdleList {
            items: Mutex::new(Vec::new()),
            max,
        }
    }

    pub fn take(&self) -> Option<Idle<C>> {
        lock(&self.items).pop()
    }

    pub fn give(&self, idle: Idle<C>) {
        let mut items = lock(&self.items);
        items.push(idle);
        if items.len() > self.max {
            items.remove(0);
        }
    }

    pub fn clear(&self) {
        lock(&self.items).clear();
    }
}

/// How a connection is thrown away when a statement was abandoned mid-way
/// (pending result data would otherwise have to be drained before reuse).
pub trait Discard: Send + 'static {
    fn discard(self);
}

impl<DB: sqlx::Database> Discard for sqlx::pool::PoolConnection<DB> {
    fn discard(self) {
        drop(self.detach());
    }
}

type AbandonHook = Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>;

/// A connection checked out for one statement. `release` parks it for reuse; if it is
/// dropped instead (the cursor was dropped mid-query) the abandon hook runs (server-side
/// cancel) and the connection is closed rather than returned to the pool.
pub struct Lease<C: Discard> {
    conn: Option<C>,
    pub id: u64,
    pub db: Option<String>,
    home: Arc<IdleList<C>>,
    abandon: Option<AbandonHook>,
}

impl<C: Discard> Lease<C> {
    pub fn new(conn: C, id: u64, db: Option<String>, home: Arc<IdleList<C>>) -> Self {
        Lease {
            conn: Some(conn),
            id,
            db,
            home,
            abandon: None,
        }
    }

    pub fn from_idle(idle: Idle<C>, home: Arc<IdleList<C>>) -> Self {
        Lease::new(idle.conn, idle.id, idle.db, home)
    }

    pub fn on_abandon(&mut self, hook: impl FnOnce() -> BoxFuture<'static, ()> + Send + 'static) {
        self.abandon = Some(Box::new(hook));
    }

    pub fn conn(&mut self) -> &mut C {
        self.conn.as_mut().expect("lease already released")
    }

    /// The statement finished cleanly: park the connection for the next one.
    pub fn release(mut self) {
        self.abandon = None;
        if let Some(conn) = self.conn.take() {
            self.home.give(Idle {
                conn,
                id: self.id,
                db: self.db.take(),
                since: Instant::now(),
            });
        }
    }

    /// The connection can't be reused (I/O error, abandoned stream): close it.
    pub fn discard(mut self) {
        self.abandon = None;
        if let Some(conn) = self.conn.take() {
            conn.discard();
        }
    }
}

impl<C: Discard> Drop for Lease<C> {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        let hook = self.abandon.take();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    if let Some(hook) = hook {
                        hook().await;
                    }
                    conn.discard();
                });
            }
            Err(_) => conn.discard(),
        }
    }
}

/// Whether an error leaves the connection usable (a SQL error does, a broken socket does not).
pub fn connection_survives(err: &Error) -> bool {
    matches!(err, Error::Database { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Filter, FilterOp, Sort};

    const PG: Dialect = Dialect::Postgres;

    fn table() -> TableRef {
        TableRef {
            database: "db".into(),
            schema: Some("public".into()),
            name: "t".into(),
        }
    }

    fn request() -> BrowseRequest {
        BrowseRequest {
            table: table(),
            filters: vec![],
            match_all: true,
            raw_where: None,
            sort: vec![],
            columns: vec![],
        }
    }

    #[test]
    fn plain_select_orders_by_key() {
        let sql = select_sql(PG, &request(), &["id".into()], Some((100, 200))).unwrap();
        assert_eq!(
            sql,
            "SELECT * FROM \"public\".\"t\" ORDER BY \"id\" LIMIT 100 OFFSET 200"
        );
    }

    #[test]
    fn explicit_sort_replaces_key_order() {
        let mut r = request();
        r.sort = vec![
            Sort {
                column: "a".into(),
                descending: true,
            },
            Sort {
                column: "b".into(),
                descending: false,
            },
        ];
        r.columns = vec!["a".into(), "b".into()];
        let sql = select_sql(PG, &r, &["id".into()], None).unwrap();
        assert_eq!(
            sql,
            "SELECT \"a\", \"b\" FROM \"public\".\"t\" ORDER BY \"a\" DESC, \"b\" ASC"
        );
    }

    #[test]
    fn filters_and_raw_where_combine() {
        let mut r = request();
        r.filters = vec![
            Filter {
                column: "a".into(),
                op: FilterOp::Eq,
                value: Some("1".into()),
            },
            Filter {
                column: "b".into(),
                op: FilterOp::IsNull,
                value: None,
            },
        ];
        r.match_all = false;
        r.raw_where = Some("WHERE c > 5 -- comment".into());
        let w = where_sql(PG, &r).unwrap().unwrap();
        assert_eq!(
            w,
            "(\"a\" = '1' OR \"b\" IS NULL) AND (\nc > 5 -- comment\n)"
        );
        assert!(!is_unfiltered(&r));
        r.filters.clear();
        r.raw_where = Some("   ".into());
        assert!(is_unfiltered(&r));
        assert_eq!(
            count_sql(PG, &r).unwrap(),
            "SELECT COUNT(*) FROM \"public\".\"t\""
        );
    }

    #[test]
    fn raw_where_keyword_is_only_stripped_as_a_word() {
        let mut r = request();
        r.raw_where = Some("whereabouts = 1".into());
        assert_eq!(raw_where(&r), Some("whereabouts = 1"));
        r.raw_where = Some("where(a = 1)".into());
        assert_eq!(raw_where(&r), Some("(a = 1)"));
    }

    #[test]
    fn insert_batches_by_rows() {
        let rows: Vec<Vec<Option<String>>> =
            (0..1201).map(|i| vec![Some(i.to_string()), None]).collect();
        let stmts = insert_statements(PG, &table(), &["a".into(), "b".into()], &rows).unwrap();
        assert_eq!(stmts.len(), 3);
        assert!(stmts[0].starts_with(
            "INSERT INTO \"public\".\"t\" (\"a\", \"b\") VALUES ('0', NULL), ('1', NULL)"
        ));
        assert_eq!(stmts[2].matches("), (").count(), 200);
        assert!(insert_statements(PG, &table(), &[], &rows).is_err());
        let ragged = vec![vec![Some("x".to_owned())]];
        assert!(insert_statements(PG, &table(), &["a".into(), "b".into()], &ragged).is_err());
    }

    #[test]
    fn insert_batches_by_size() {
        let big = "x".repeat(400 * 1024);
        let rows: Vec<Vec<Option<String>>> = (0..5).map(|_| vec![Some(big.clone())]).collect();
        let stmts = insert_statements(PG, &table(), &["a".into()], &rows).unwrap();
        assert!(stmts.len() >= 2);
        assert!(
            stmts
                .iter()
                .all(|s| s.len() <= INSERT_BATCH_BYTES + 400 * 1024)
        );
    }

    #[test]
    fn editability_reasons() {
        assert_eq!(
            editability(TableKind::View, &["id".into()]),
            Editability::ReadOnly("Views can't be edited.".into())
        );
        assert_eq!(
            editability(TableKind::Table, &[]),
            Editability::ReadOnly(NO_PK_REASON.into())
        );
        assert_eq!(
            editability(TableKind::Table, &["id".into()]),
            Editability::Editable
        );
    }

    #[test]
    fn structure_cache_expires_and_tracks_keys() {
        let cache = StructureCache::default();
        let t = table();
        assert!(cache.primary_key(&t).is_none());
        cache.put_primary_key(&t, vec!["id".into()]);
        assert_eq!(cache.primary_key(&t).unwrap().as_slice(), ["id".to_owned()]);
        cache.clear();
        assert!(cache.primary_key(&t).is_none());
    }

    #[test]
    fn cancel_state_maps_errors_only_after_a_request() {
        let s = CancelState::default();
        assert!(!s.request());
        s.start();
        assert!(matches!(
            s.map_error(Error::database("x")),
            Error::Database { .. }
        ));
        assert!(s.request());
        assert!(matches!(
            s.map_error(Error::database("x")),
            Error::Cancelled
        ));
        s.start();
        assert!(!s.requested.load(Ordering::SeqCst));
    }
}
