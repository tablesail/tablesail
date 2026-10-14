//! The `Driver` trait every engine implements, and the streaming `Cursor`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::Engine;
use crate::dialect::Dialect;
use crate::error::{Error, Result};
use crate::model::{
    ApplyResult, BrowseRequest, ChangeSet, ColumnMeta, Page, RowCount, TableInfo, TableRef,
    TableStructure,
};
use crate::value::Row;

/// One open connection (usually a small pool) to a server or file.
///
/// Every method is cancel-safe: dropping the future must not leave the
/// connection unusable. Long operations that the user can cancel go through
/// `query`/`browse_all`, which return a `Cursor` with a `CancelHandle`.
#[async_trait]
pub trait Driver: Send + Sync {
    fn engine(&self) -> Engine;

    fn dialect(&self) -> Dialect;

    /// e.g. "PostgreSQL 17.2", "8.4.3-MySQL", "11.8.2-MariaDB", "MongoDB 8.2.1", "SQLite 3.50.1".
    async fn server_version(&self) -> Result<String>;

    /// Databases visible to the user. SQLite: `["main"]` plus attached databases.
    async fn list_databases(&self) -> Result<Vec<String>>;

    /// Schemas within a database (Postgres). Engines without schemas return an empty list.
    async fn list_schemas(&self, database: &str) -> Result<Vec<String>>;

    /// Tables, views and collections. `schema` is None for engines without schemas.
    /// System schemas/tables (pg_catalog, information_schema, sqlite_*, FTS shadow
    /// tables, system.*) are excluded.
    async fn list_tables(&self, database: &str, schema: Option<&str>) -> Result<Vec<TableInfo>>;

    /// Columns, keys, indexes, foreign keys, editability and DDL of one table.
    /// Mongo samples documents to infer columns.
    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure>;

    /// One page of a table with filters and sorting pushed down to the server.
    /// Without an explicit sort the driver orders by primary key when one exists,
    /// so pages are stable. Values are passed through `Value::for_grid`.
    async fn browse_page(&self, request: &BrowseRequest, offset: u64, limit: u32) -> Result<Page>;

    /// Row count for the same request. For unfiltered tables larger than ~1M rows,
    /// drivers may return only `estimate` to stay fast.
    async fn count_rows(&self, request: &BrowseRequest) -> Result<RowCount>;

    /// Streams every row matching `request` (used by export). Values are complete (not truncated).
    async fn browse_all(&self, request: &BrowseRequest) -> Result<Box<dyn Cursor>>;

    /// Runs ONE statement (scripts are split by the caller with `split_statements`)
    /// on a dedicated connection, in `database` when given. The statement may
    /// still produce several result sets (MySQL CALL). Grid truncation applies.
    async fn query(&self, database: Option<&str>, sql: &str) -> Result<Box<dyn Cursor>>;

    /// SQL (or Mongo commands as JSON) that `apply_changes` would run, for preview.
    async fn preview_changes(&self, changes: &ChangeSet) -> Result<Vec<String>>;

    /// Applies all changes in one transaction; any error rolls everything back and
    /// returns the server's message with the failing statement attached.
    async fn apply_changes(&self, changes: &ChangeSet) -> Result<ApplyResult>;

    /// Inserts rows (CSV import) in batches inside one transaction. `columns` are target names.
    async fn insert_rows(
        &self,
        table: &TableRef,
        columns: &[String],
        rows: Vec<Vec<Option<String>>>,
    ) -> Result<u64>;

    /// A driver bound to ONE dedicated connection, in `database` when given (default: the
    /// connection's own), for a manual transaction: BEGIN, the statements and COMMIT/ROLLBACK
    /// all land on that connection. It goes through the SSH tunnel like the driver it comes
    /// from, is independent of it (closing one leaves the other open), and statements on it
    /// take turns. When its connection is lost it does not reconnect, since a new connection
    /// would no longer hold the transaction. Dropping a cursor of it finishes reading the
    /// statement in the background rather than discarding the connection. Only `query`,
    /// `transaction_aborted` and `close` are meant to be used on it.
    async fn pinned(&self, _database: Option<&str>) -> Result<Arc<dyn Driver>> {
        Err(Error::InvalidInput(
            "Manual transactions aren't supported for this connection.".to_owned(),
        ))
    }

    /// Whether the transaction open on this (pinned) driver has failed and can only be rolled
    /// back. Postgres ends a transaction at the first error but keeps accepting COMMIT, which
    /// then silently rolls back; other engines have no such state.
    async fn transaction_aborted(&self) -> Result<bool> {
        Ok(false)
    }

    /// Forgets the cached table structures and keys. A transaction that changed the schema
    /// calls this on the driver it was opened from once it has ended, since that driver
    /// never saw the statements.
    fn forget_structures(&self) {}

    /// Closes pools. Further calls may fail.
    async fn close(&self);
}

/// Events produced by a running statement, in order:
/// `Columns`, then zero or more `Rows`, then optionally `Affected`; repeated per
/// result set; the cursor returns `None` once the statement is finished.
#[derive(Debug, Clone, PartialEq)]
pub enum CursorEvent {
    /// A new result set starts.
    Columns(Vec<ColumnMeta>),
    Rows(Vec<Row>),
    /// Rows affected by a statement that returns no rows (or after a result set).
    Affected(u64),
}

/// A streaming result. Implementations run the statement in a background task
/// that owns its connection and pushes batches into a bounded channel, so memory
/// stays bounded and the first batch arrives as soon as the server sends it.
#[async_trait]
pub trait Cursor: Send {
    /// Next event, or `Ok(None)` when finished. Batches hold at most `max_rows` rows.
    async fn next(&mut self, max_rows: usize) -> Result<Option<CursorEvent>>;

    /// Handle that cancels the statement server-side (Postgres `pg_cancel_backend`,
    /// MySQL `KILL QUERY`, Mongo `killOp`/cursor kill; SQLite interrupts). Usable
    /// from another task while `next` is pending.
    fn cancel_handle(&self) -> Arc<dyn CancelHandle>;
}

#[async_trait]
pub trait CancelHandle: Send + Sync {
    async fn cancel(&self) -> Result<()>;
}
