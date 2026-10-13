//! SQLite driver. Values keep their storage class (INTEGER/REAL/TEXT/BLOB/NULL);
//! the declared column type only decides the column family.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{FutureExt, StreamExt};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::{Sqlite, SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::{
    AssertSqlSafe, Column, Either, Executor, Row, SqlSafeStr, Statement, TypeInfo, ValueRef,
};

use super::sqlcommon::{
    CancelState, IdleList, Lease, Pump, StructureCache, apply_in_transaction, connection_survives,
    count_sql, editability, insert_in_transaction, insert_statements, meta, select_sql,
};
use crate::Engine;
use crate::changes::change_statements;
use crate::cursor::ChannelCursor;
use crate::dialect::Dialect;
use crate::driver::{CancelHandle, Cursor, Driver};
use crate::error::{Error, Result};
use crate::model::{
    ApplyResult, BrowseRequest, ChangeSet, ColumnInfo, ColumnMeta, ConnectParams, ForeignKeyInfo,
    IndexInfo, Page, RowCount, StatementKind, TableInfo, TableKind, TableRef, TableStructure,
    TypeFamily,
};
use crate::querylog::logged;
use crate::split::statement_kind;
use crate::value::{Row as DbRow, Value};

const DIALECT: Dialect = Dialect::Sqlite;

unsafe extern "C" {
    /// Thread-safe; makes the running statement on that connection fail with SQLITE_INTERRUPT.
    fn sqlite3_interrupt(db: *mut std::ffi::c_void);
}

pub async fn connect(params: &ConnectParams) -> Result<Arc<dyn Driver>> {
    let path = params
        .config
        .file_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| Error::InvalidInput("Choose a SQLite database file.".to_owned()))?;
    let memory = path == ":memory:";
    let mut options = if memory {
        // Parsing gives every pool its own named shared-cache database; all its
        // connections see the same data.
        "sqlite::memory:"
            .parse::<SqliteConnectOptions>()
            .map_err(Error::from)?
    } else {
        if !Path::new(path).is_file() {
            return Err(Error::Connection(format!(
                "The database file doesn't exist: {path}"
            )));
        }
        SqliteConnectOptions::new().filename(path)
    };
    options = options.create_if_missing(false);
    let mut pool_options = SqlitePoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(15));
    if memory {
        // The in-memory database disappears with its last connection.
        pool_options = pool_options
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None);
    }
    let pool = pool_options.connect_with(options).await?;

    // Reading the schema forces SQLite to validate the header ("file is not a database").
    let version: String = {
        let row = sqlx::raw_sql(logged(
            "SELECT sqlite_version(), (SELECT count(*) FROM sqlite_master)",
        ))
        .fetch_one(&pool)
        .await?;
        sqlite_text(&row, 0)?.unwrap_or_default()
    };
    Ok(Arc::new(SqliteDriver {
        pool,
        version: format!("SQLite {version}"),
        idle: Arc::new(IdleList::new(2)),
        cache: StructureCache::default(),
    }))
}

struct SqliteDriver {
    pool: SqlitePool,
    version: String,
    idle: Arc<IdleList<PoolConnection<Sqlite>>>,
    cache: StructureCache,
}

type SqliteLease = Lease<PoolConnection<Sqlite>>;

/// Cancel handle for one statement. `live` is true while the statement owns its
/// connection, which keeps the raw handle pointer valid for `fire`.
struct Interrupt {
    ptr: u64,
    live: Mutex<bool>,
    state: CancelState,
}

impl Interrupt {
    fn new(ptr: u64) -> Arc<Self> {
        let i = Arc::new(Interrupt {
            ptr,
            live: Mutex::new(true),
            state: CancelState::default(),
        });
        i.state.start();
        i
    }

    fn fire(&self) {
        let live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        if *live && self.state.request() {
            // SAFETY: `live` is only true while the connection is checked out to the
            // statement, and `stop` (which precedes releasing it) takes the same lock.
            unsafe { sqlite3_interrupt(self.ptr as usize as *mut std::ffi::c_void) };
        }
    }

    fn stop(&self) {
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        *live = false;
        self.state.finish();
    }
}

#[async_trait]
impl CancelHandle for Interrupt {
    async fn cancel(&self) -> Result<()> {
        self.fire();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// A result column plus what decoding needs to know about it.
#[derive(Clone)]
struct Col {
    meta: ColumnMeta,
    /// Declared BOOLEAN: 0/1 integers show as booleans.
    bool_col: bool,
}

fn family_of_decl(decl: &str) -> TypeFamily {
    let t = decl.to_ascii_lowercase();
    if t.contains("bool") {
        TypeFamily::Boolean
    } else if t.contains("timestamp") || t.contains("datetime") {
        TypeFamily::DateTime
    } else if t.contains("date") {
        TypeFamily::Date
    } else if t.starts_with("time") {
        TypeFamily::Time
    } else if t.contains("json") {
        TypeFamily::Json
    } else if t.contains("uuid") {
        TypeFamily::Uuid
    } else if t.contains("int") {
        TypeFamily::Integer
    } else if t.contains("char") || t.contains("clob") || t.contains("text") {
        TypeFamily::Text
    } else if t.contains("blob") {
        TypeFamily::Binary
    } else if t.contains("real") || t.contains("floa") || t.contains("doub") {
        TypeFamily::Float
    } else if t.contains("dec") || t.contains("num") {
        TypeFamily::Decimal
    } else {
        TypeFamily::Other
    }
}

/// Family and type name for a storage class reported by SQLite.
fn storage_family(class: &str) -> (TypeFamily, &'static str) {
    match class {
        "INTEGER" => (TypeFamily::Integer, "INTEGER"),
        "REAL" => (TypeFamily::Float, "REAL"),
        "TEXT" => (TypeFamily::Text, "TEXT"),
        "BLOB" => (TypeFamily::Binary, "BLOB"),
        _ => (TypeFamily::Other, "NULL"),
    }
}

/// Builds `Col`s from sqlx's coarse declared types; expression columns (no declared
/// type) take their family from the first row's value.
fn columns_of<'a>(
    cols: impl Iterator<Item = (&'a str, &'a str)>,
    first: Option<&SqliteRow>,
) -> Vec<Col> {
    cols.enumerate()
        .map(|(i, (name, declared))| {
            let (family, type_name) = if declared == "NULL" {
                match first.and_then(|r| r.try_get_raw(i).ok()) {
                    Some(v) if !v.is_null() => {
                        let (f, n) = storage_family(v.type_info().name());
                        (f, n.to_owned())
                    }
                    _ => (TypeFamily::Other, "NULL".to_owned()),
                }
            } else {
                (family_of_decl(declared), declared.to_owned())
            };
            Col {
                meta: meta(name, type_name, family),
                bool_col: declared == "BOOLEAN",
            }
        })
        .collect()
}

fn columns_of_row(row: &SqliteRow) -> Vec<Col> {
    columns_of(
        row.columns()
            .iter()
            .map(|c| (c.name(), c.type_info().name())),
        Some(row),
    )
}

fn decode_row(row: &SqliteRow, cols: &[Col], grid: bool) -> Result<DbRow> {
    let mut out = Vec::with_capacity(cols.len());
    for (i, col) in cols.iter().enumerate() {
        let v = decode_value(row, i, col.bool_col)?;
        out.push(if grid { v.for_grid() } else { v });
    }
    Ok(out)
}

fn decode_value(row: &SqliteRow, i: usize, bool_col: bool) -> Result<Value> {
    let raw = row.try_get_raw(i)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    let class = raw.type_info().name().to_owned();
    drop(raw);
    Ok(match class.as_str() {
        "INTEGER" => {
            let n: i64 = row.try_get_unchecked(i)?;
            if bool_col && (n == 0 || n == 1) {
                Value::Bool(n == 1)
            } else {
                Value::Int(n)
            }
        }
        "REAL" => Value::Float(row.try_get_unchecked(i)?),
        "BLOB" => Value::bytes(row.try_get_unchecked::<Vec<u8>, _>(i)?),
        _ => match row.try_get_unchecked::<String, _>(i) {
            Ok(s) => Value::Text(s),
            // Text that is not valid UTF-8: show what can be shown.
            Err(_) => Value::Text(
                String::from_utf8_lossy(&row.try_get_unchecked::<Vec<u8>, _>(i)?).into_owned(),
            ),
        },
    })
}

/// Any cell as text (introspection queries).
fn sqlite_text(row: &SqliteRow, i: usize) -> Result<Option<String>> {
    Ok(match decode_value(row, i, false)? {
        Value::Null => None,
        Value::Int(n) => Some(n.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Text(s) => Some(s),
        Value::Bytes { data, .. } => Some(hex::encode(data)),
        other => Some(other.to_display()),
    })
}

type TextRows = Vec<Vec<Option<String>>>;

fn cell(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

fn db_name(database: &str) -> &str {
    if database.is_empty() {
        "main"
    } else {
        database
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

impl SqliteDriver {
    async fn text_rows(&self, sql: &str) -> Result<TextRows> {
        let rows = sqlx::raw_sql(logged(sql)).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| (0..r.len()).map(|i| sqlite_text(r, i)).collect())
            .collect()
    }

    async fn lease(&self) -> Result<SqliteLease> {
        if let Some(idle) = self.idle.take() {
            return Ok(Lease::from_idle(idle, self.idle.clone()));
        }
        let mut conn = self.pool.acquire().await?;
        let ptr = {
            let mut handle = conn.lock_handle().await?;
            handle.as_raw_handle().as_ptr() as usize as u64
        };
        Ok(Lease::new(conn, ptr, None, self.idle.clone()))
    }

    async fn start(&self, sql: &str, grid: bool) -> Result<Box<dyn Cursor>> {
        let mut lease = self.lease().await?;
        let kind = statement_kind(sql, DIALECT);
        let interrupt = Interrupt::new(lease.id);
        lease.on_abandon({
            let i = interrupt.clone();
            move || {
                async move {
                    i.fire();
                    i.stop();
                }
                .boxed()
            }
        });
        let sql = sql.to_owned();
        let cancel: Arc<dyn CancelHandle> = interrupt.clone();
        Ok(Box::new(ChannelCursor::spawn(cancel, move |tx| {
            run_statement(lease, tx, sql, grid, kind, interrupt)
        })))
    }

    async fn primary_key(&self, table: &TableRef) -> Result<Arc<Vec<String>>> {
        if let Some(k) = self.cache.primary_key(table) {
            return Ok(k);
        }
        let rows = self
            .text_rows(&format!(
                "PRAGMA {}.table_info({})",
                DIALECT.quote_ident(db_name(&table.database)),
                DIALECT.quote_string(&table.name)
            ))
            .await?;
        Ok(self.cache.put_primary_key(table, primary_key_of(&rows)))
    }

    async fn structure(&self, table: &TableRef) -> Result<Arc<TableStructure>> {
        if let Some(s) = self.cache.structure(table) {
            return Ok(s);
        }
        let s = self.read_structure(table).await?;
        Ok(self.cache.put_structure(s))
    }

    async fn read_structure(&self, table: &TableRef) -> Result<TableStructure> {
        let db = db_name(&table.database).to_owned();
        let qdb = DIALECT.quote_ident(&db);
        let qname = DIALECT.quote_string(&table.name);

        let master = self
            .text_rows(&format!(
                "SELECT type, sql FROM {qdb}.sqlite_master WHERE name = {qname} AND type IN ('table', 'view')"
            ))
            .await?;
        let Some(master) = master.first() else {
            return Err(Error::NotFound(format!("table {}", table.name)));
        };
        let ddl = master.get(1).cloned().flatten();
        let is_view = cell(master, 0) == "view";
        let is_virtual = ddl.as_deref().is_some_and(is_virtual_ddl);
        let kind = if is_view {
            TableKind::View
        } else if is_virtual {
            TableKind::Virtual
        } else {
            TableKind::Table
        };
        let without_rowid = ddl
            .as_deref()
            .is_some_and(|d| d.to_ascii_uppercase().contains("WITHOUT ROWID"));

        // cid, name, type, notnull, dflt_value, pk, hidden
        let info = self
            .text_rows(&format!("PRAGMA {qdb}.table_xinfo({qname})"))
            .await?;
        let mut pk_positions: Vec<(i64, String)> = Vec::new();
        let mut columns = Vec::new();
        for r in &info {
            let hidden: i64 = cell(r, 6).parse().unwrap_or(0);
            if hidden == 1 {
                continue; // hidden column of a virtual table
            }
            let name = cell(r, 1);
            let declared = cell(r, 2);
            let pk: i64 = cell(r, 5).parse().unwrap_or(0);
            if pk > 0 {
                pk_positions.push((pk, name.clone()));
            }
            columns.push((
                name,
                declared,
                cell(r, 3) == "1",
                r.get(4).cloned().flatten(),
                pk,
                hidden,
            ));
        }
        pk_positions.sort();
        let primary_key: Vec<String> = pk_positions.into_iter().map(|(_, n)| n).collect();
        let rowid_alias = primary_key.len() == 1 && !without_rowid && !is_view && {
            columns
                .iter()
                .any(|c| c.4 > 0 && c.1.eq_ignore_ascii_case("INTEGER"))
        };

        let columns: Vec<ColumnInfo> = columns
            .into_iter()
            .map(|(name, declared, notnull, default, pk, hidden)| {
                let alias = rowid_alias && pk > 0;
                ColumnInfo {
                    family: family_of_decl(&declared),
                    data_type: declared,
                    nullable: !notnull && !alias,
                    default,
                    is_primary_key: pk > 0,
                    is_auto_increment: alias,
                    is_generated: hidden >= 2,
                    enum_values: None,
                    comment: None,
                    name,
                }
            })
            .collect();

        // seq, name, unique, origin, partial
        let index_rows = if is_view {
            Vec::new()
        } else {
            self.text_rows(&format!("PRAGMA {qdb}.index_list({qname})"))
                .await?
        };
        let mut indexes = Vec::new();
        for r in &index_rows {
            let name = cell(r, 1);
            let cols = self
                .text_rows(&format!(
                    "PRAGMA {qdb}.index_info({})",
                    DIALECT.quote_string(&name)
                ))
                .await?
                .iter()
                .filter_map(|c| c.get(2).cloned().flatten())
                .collect();
            indexes.push(IndexInfo {
                name,
                columns: cols,
                unique: cell(r, 2) == "1",
                primary: cell(r, 3) == "pk",
                method: Some("btree".to_owned()),
            });
        }
        if !primary_key.is_empty() && !indexes.iter().any(|i| i.primary) {
            // The rowid alias has no index of its own.
            indexes.insert(
                0,
                IndexInfo {
                    name: "PRIMARY".to_owned(),
                    columns: primary_key.clone(),
                    unique: true,
                    primary: true,
                    method: None,
                },
            );
        }

        // id, seq, table, from, to, on_update, on_delete, match
        let fk_rows = if is_view {
            Vec::new()
        } else {
            self.text_rows(&format!("PRAGMA {qdb}.foreign_key_list({qname})"))
                .await?
        };
        let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        for r in &fk_rows {
            let id = cell(r, 0);
            let to = r.get(4).cloned().flatten();
            let idx = match ids.iter().position(|i| *i == id) {
                Some(idx) => idx,
                None => {
                    ids.push(id.clone());
                    foreign_keys.push(ForeignKeyInfo {
                        name: format!("fk_{}_{id}", table.name),
                        columns: vec![],
                        referenced: TableRef {
                            database: db.clone(),
                            schema: None,
                            name: cell(r, 2),
                        },
                        referenced_columns: vec![],
                        on_update: Some(cell(r, 5)).filter(|a| !a.is_empty()),
                        on_delete: Some(cell(r, 6)).filter(|a| !a.is_empty()),
                    });
                    ids.len() - 1
                }
            };
            foreign_keys[idx].columns.push(cell(r, 3));
            if let Some(to) = to {
                foreign_keys[idx].referenced_columns.push(to);
            }
        }
        // `REFERENCES t` without columns means t's primary key.
        for fk in &mut foreign_keys {
            if fk.referenced_columns.is_empty() {
                let rows = self
                    .text_rows(&format!(
                        "PRAGMA {qdb}.table_info({})",
                        DIALECT.quote_string(&fk.referenced.name)
                    ))
                    .await
                    .unwrap_or_default();
                fk.referenced_columns = primary_key_of(&rows);
            }
        }

        Ok(TableStructure {
            table: TableRef {
                database: db,
                schema: None,
                name: table.name.clone(),
            },
            kind,
            editability: editability(kind, &primary_key),
            columns,
            primary_key,
            indexes,
            foreign_keys,
            ddl,
        })
    }
}

fn primary_key_of(table_info_rows: &TextRows) -> Vec<String> {
    // cid, name, type, notnull, dflt_value, pk
    let mut pk: Vec<(i64, String)> = table_info_rows
        .iter()
        .filter_map(|r| {
            let pos: i64 = cell(r, 5).parse().ok()?;
            (pos > 0).then(|| (pos, cell(r, 1)))
        })
        .collect();
    pk.sort();
    pk.into_iter().map(|(_, n)| n).collect()
}

fn is_virtual_ddl(ddl: &str) -> bool {
    let mut words = ddl.split_whitespace();
    matches!(
        (words.next(), words.next()),
        (Some(a), Some(b)) if a.eq_ignore_ascii_case("create") && b.eq_ignore_ascii_case("virtual")
    )
}

/// Suffixes of the hidden tables FTS3/4/5 and R-Tree keep next to a virtual table.
const SHADOW_SUFFIXES: [&str; 11] = [
    "data", "idx", "content", "docsize", "config", "segdir", "segments", "stat", "node", "rowid",
    "parent",
];

fn is_shadow_table(name: &str, virtual_tables: &[String]) -> bool {
    virtual_tables.iter().any(|vt| {
        name.strip_prefix(vt.as_str())
            .and_then(|rest| rest.strip_prefix('_'))
            .is_some_and(|suffix| SHADOW_SUFFIXES.contains(&suffix))
    })
}

#[async_trait]
impl Driver for SqliteDriver {
    fn engine(&self) -> Engine {
        Engine::Sqlite
    }

    fn dialect(&self) -> Dialect {
        DIALECT
    }

    async fn server_version(&self) -> Result<String> {
        Ok(self.version.clone())
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        let rows = self.text_rows("PRAGMA database_list").await?;
        Ok(rows
            .iter()
            .map(|r| cell(r, 1))
            .filter(|n| !n.is_empty() && n != "temp")
            .collect())
    }

    async fn list_schemas(&self, _database: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn list_tables(&self, database: &str, _schema: Option<&str>) -> Result<Vec<TableInfo>> {
        let db = db_name(database).to_owned();
        let rows = self
            .text_rows(&format!(
                "SELECT name, type, sql FROM {}.sqlite_master \
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
                 ORDER BY name",
                DIALECT.quote_ident(&db)
            ))
            .await?;
        let virtual_tables: Vec<String> = rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .cloned()
                    .flatten()
                    .as_deref()
                    .is_some_and(is_virtual_ddl)
            })
            .map(|r| cell(r, 0))
            .collect();
        Ok(rows
            .iter()
            .filter_map(|r| {
                let name = cell(r, 0);
                if is_shadow_table(&name, &virtual_tables) {
                    return None;
                }
                let kind = if cell(r, 1) == "view" {
                    TableKind::View
                } else if virtual_tables.contains(&name) {
                    TableKind::Virtual
                } else {
                    TableKind::Table
                };
                Some(TableInfo {
                    table: TableRef {
                        database: db.clone(),
                        schema: None,
                        name,
                    },
                    kind,
                    estimated_rows: None,
                    comment: None,
                })
            })
            .collect())
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        let s = self.read_structure(table).await?;
        Ok((*self.cache.put_structure(s)).clone())
    }

    async fn browse_page(&self, request: &BrowseRequest, offset: u64, limit: u32) -> Result<Page> {
        let started = Instant::now();
        let key = if request.sort.is_empty() {
            self.primary_key(&request.table).await?
        } else {
            Arc::new(Vec::new())
        };
        let sql = select_sql(DIALECT, request, &key, Some((limit, offset)))?;
        let rows = sqlx::raw_sql(logged(sql.clone()))
            .fetch_all(&self.pool)
            .await?;
        let cols = match rows.first() {
            Some(first) => columns_of_row(first),
            None => {
                let stmt = (&self.pool)
                    .prepare(AssertSqlSafe(sql.clone()).into_sql_str())
                    .await?;
                columns_of(
                    stmt.columns()
                        .iter()
                        .map(|c| (c.name(), c.type_info().name())),
                    None,
                )
            }
        };
        let rows = rows
            .iter()
            .map(|r| decode_row(r, &cols, true))
            .collect::<Result<Vec<_>>>()?;
        Ok(Page {
            columns: cols.into_iter().map(|c| c.meta).collect(),
            rows,
            offset,
            query: sql,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn count_rows(&self, request: &BrowseRequest) -> Result<RowCount> {
        let sql = count_sql(DIALECT, request)?;
        let row = sqlx::raw_sql(logged(sql)).fetch_one(&self.pool).await?;
        let n: i64 = row.try_get_unchecked(0)?;
        Ok(RowCount {
            exact: Some(n.max(0) as u64),
            estimate: None,
        })
    }

    async fn browse_all(&self, request: &BrowseRequest) -> Result<Box<dyn Cursor>> {
        let sql = select_sql(DIALECT, request, &[], None)?;
        self.start(&sql, false).await
    }

    async fn query(&self, _database: Option<&str>, sql: &str) -> Result<Box<dyn Cursor>> {
        self.start(sql, true).await
    }

    async fn preview_changes(&self, changes: &ChangeSet) -> Result<Vec<String>> {
        let structure = self.structure(&changes.table).await?;
        change_statements(DIALECT, &structure, changes)
    }

    async fn apply_changes(&self, changes: &ChangeSet) -> Result<ApplyResult> {
        let structure = self.structure(&changes.table).await?;
        let statements = change_statements(DIALECT, &structure, changes)?;
        apply_in_transaction(&self.pool, &statements, &changes.changes).await
    }

    async fn insert_rows(
        &self,
        table: &TableRef,
        columns: &[String],
        rows: Vec<Vec<Option<String>>>,
    ) -> Result<u64> {
        let statements = insert_statements(DIALECT, table, columns, &rows)?;
        insert_in_transaction(&self.pool, &statements).await
    }

    async fn close(&self) {
        self.idle.clear();
        self.pool.close().await;
    }
}

/// The producer task of one statement.
async fn run_statement(
    mut lease: SqliteLease,
    tx: crate::cursor::EventSender,
    sql: String,
    grid: bool,
    kind: StatementKind,
    interrupt: Arc<Interrupt>,
) -> Result<()> {
    let mut pump = Pump::new(tx, grid);
    let outcome = stream_statement(&mut lease, &sql, kind, &mut pump).await;
    interrupt.stop();
    match outcome {
        Ok(()) => {
            lease.release();
            Ok(())
        }
        Err(err) => {
            let err = if interrupt.state.requested.load(Ordering::SeqCst) {
                Error::Cancelled
            } else {
                err
            };
            if connection_survives(&err) {
                lease.release();
            } else {
                lease.discard();
            }
            Err(err)
        }
    }
}

async fn stream_statement(
    lease: &mut SqliteLease,
    sql: &str,
    kind: StatementKind,
    pump: &mut Pump,
) -> Result<()> {
    let conn = &mut **lease.conn();
    let mut cols: Vec<Col> = Vec::new();
    let mut deferred: Option<u64> = None;
    {
        let mut stream = sqlx::raw_sql(logged(sql)).fetch_many(&mut *conn);
        while let Some(item) = stream.next().await {
            match item? {
                Either::Right(row) => {
                    if !pump.in_set() {
                        cols = columns_of_row(&row);
                        pump.columns(cols.iter().map(|c| c.meta.clone()).collect())
                            .await?;
                    }
                    pump.row(decode_row(&row, &cols, pump.grid())?).await?;
                }
                Either::Left(result) => {
                    // SQLite reports the count of the last DML statement for anything else.
                    let affected = if kind == StatementKind::Write {
                        result.rows_affected()
                    } else {
                        0
                    };
                    if let Some(n) = pump.end_set(affected, kind == StatementKind::Read).await? {
                        deferred = Some(n);
                    }
                }
            }
        }
    }
    if let Some(affected) = deferred {
        // A query without rows still has columns; ask SQLite for them.
        let described = (&mut *conn)
            .prepare(AssertSqlSafe(sql.to_owned()).into_sql_str())
            .await
            .ok()
            .map(|stmt| {
                columns_of(
                    stmt.columns()
                        .iter()
                        .map(|c| (c.name(), c.type_info().name())),
                    None,
                )
            })
            .filter(|c| !c.is_empty());
        match described {
            Some(cols) => {
                pump.columns(cols.into_iter().map(|c| c.meta).collect())
                    .await?;
                pump.end_set(0, false).await?;
            }
            None => pump.affected(affected).await?,
        }
    }
    pump.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::CursorEvent;
    use crate::model::{
        ColumnInput, ColumnValue, ConnectionConfig, Editability, EnvTag, Filter, FilterOp,
        RowChange, Sort,
    };
    use crate::value::CellInput;
    use std::collections::BTreeMap;

    const SCHEMA: &str = "
        CREATE TABLE people (
            id INTEGER PRIMARY KEY,
            name VARCHAR(40) NOT NULL DEFAULT 'anon',
            age INT,
            active BOOLEAN,
            score REAL,
            photo BLOB,
            created DATETIME,
            full_name TEXT GENERATED ALWAYS AS (name || '!') VIRTUAL
        );
        CREATE TABLE no_pk (a TEXT, b TEXT);
        CREATE TABLE pairs (x INT, y INT, label TEXT, PRIMARY KEY (x, y));
        CREATE TABLE orders (id INTEGER PRIMARY KEY, person INT REFERENCES people(id) ON DELETE CASCADE);
        CREATE INDEX people_age ON people(age);
        CREATE VIEW adults AS SELECT * FROM people WHERE age >= 18;
        CREATE VIRTUAL TABLE docs USING fts5(body);
        INSERT INTO people (id, name, age, active, score, photo, created)
          VALUES (1, 'ann', 30, 1, 1.5, X'DEAD', '2024-01-01 10:00:00'),
                 (2, 'bob', NULL, 0, NULL, NULL, NULL),
                 (3, 'o''neil', 17, NULL, 2.25, X'', '0000-00-00');
        INSERT INTO no_pk VALUES ('x', 'y');
        INSERT INTO docs (body) VALUES ('hello world');
    ";

    fn config_for(path: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: String::new(),
            name: "test".into(),
            engine: Engine::Sqlite,
            host: String::new(),
            port: 0,
            user: String::new(),
            database: String::new(),
            file_path: Some(path.into()),
            url: None,
            ssl: Default::default(),
            ssh: None,
            env: EnvTag::Local,
            color: None,
            read_only: false,
            options: BTreeMap::new(),
        }
    }

    async fn test_driver() -> Arc<dyn Driver> {
        let driver = connect(&ConnectParams {
            config: config_for(":memory:"),
            password: None,
            host: String::new(),
            port: 0,
        })
        .await
        .unwrap();
        // Seed through the driver's own query path.
        let mut cursor = driver.query(None, SCHEMA).await.unwrap();
        while cursor.next(100).await.unwrap().is_some() {}
        driver
    }

    fn table(name: &str) -> TableRef {
        TableRef {
            database: "main".into(),
            schema: None,
            name: name.into(),
        }
    }

    fn req(name: &str) -> BrowseRequest {
        BrowseRequest {
            table: table(name),
            filters: vec![],
            match_all: true,
            raw_where: None,
            sort: vec![],
            columns: vec![],
        }
    }

    async fn collect(mut cursor: Box<dyn Cursor>) -> Vec<CursorEvent> {
        let mut out = Vec::new();
        while let Some(e) = cursor.next(1000).await.unwrap() {
            out.push(e);
        }
        out
    }

    #[tokio::test]
    async fn lists_databases_and_tables_without_shadow_tables() {
        let d = test_driver().await;
        assert_eq!(d.list_databases().await.unwrap(), ["main"]);
        assert!(d.list_schemas("main").await.unwrap().is_empty());
        let tables = d.list_tables("main", None).await.unwrap();
        let names: Vec<_> = tables.iter().map(|t| t.table.name.as_str()).collect();
        assert_eq!(
            names,
            ["adults", "docs", "no_pk", "orders", "pairs", "people"]
        );
        let kind = |n: &str| tables.iter().find(|t| t.table.name == n).unwrap().kind;
        assert_eq!(kind("adults"), TableKind::View);
        assert_eq!(kind("docs"), TableKind::Virtual);
        assert_eq!(kind("people"), TableKind::Table);
        assert!(d.server_version().await.unwrap().starts_with("SQLite 3."));
    }

    #[tokio::test]
    async fn describes_a_table() {
        let d = test_driver().await;
        let s = d.table_structure(&table("people")).await.unwrap();
        assert_eq!(s.primary_key, ["id"]);
        assert_eq!(s.editability, Editability::Editable);
        let col = |n: &str| s.columns.iter().find(|c| c.name == n).unwrap();
        assert!(col("id").is_auto_increment && col("id").is_primary_key && !col("id").nullable);
        assert_eq!(col("name").family, TypeFamily::Text);
        assert_eq!(col("name").data_type, "VARCHAR(40)");
        assert!(!col("name").nullable);
        assert_eq!(col("name").default.as_deref(), Some("'anon'"));
        assert_eq!(col("active").family, TypeFamily::Boolean);
        assert_eq!(col("score").family, TypeFamily::Float);
        assert_eq!(col("photo").family, TypeFamily::Binary);
        assert_eq!(col("created").family, TypeFamily::DateTime);
        assert!(col("full_name").is_generated);
        assert!(
            s.indexes
                .iter()
                .any(|i| i.name == "people_age" && i.columns == ["age"])
        );
        assert!(s.indexes.iter().any(|i| i.primary && i.columns == ["id"]));
        assert!(s.ddl.as_deref().unwrap().starts_with("CREATE TABLE people"));

        let fk = d
            .table_structure(&table("orders"))
            .await
            .unwrap()
            .foreign_keys;
        assert_eq!(fk.len(), 1);
        assert_eq!(fk[0].columns, ["person"]);
        assert_eq!(fk[0].referenced.name, "people");
        assert_eq!(fk[0].referenced_columns, ["id"]);
        assert_eq!(fk[0].on_delete.as_deref(), Some("CASCADE"));

        let pairs = d.table_structure(&table("pairs")).await.unwrap();
        assert_eq!(pairs.primary_key, ["x", "y"]);
        assert!(!pairs.columns[0].is_auto_increment);

        let no_pk = d.table_structure(&table("no_pk")).await.unwrap();
        assert_eq!(
            no_pk.editability,
            Editability::ReadOnly(
                "This table has no primary key, so rows can't be identified safely.".into()
            )
        );
        let view = d.table_structure(&table("adults")).await.unwrap();
        assert_eq!(view.kind, TableKind::View);
        assert_eq!(
            view.editability,
            Editability::ReadOnly("Views can't be edited.".into())
        );
        assert!(matches!(
            d.table_structure(&table("nope")).await,
            Err(Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn browses_pages_in_key_order() {
        let d = test_driver().await;
        let page = d.browse_page(&req("people"), 1, 2).await.unwrap();
        assert_eq!(page.offset, 1);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0][0], Value::Int(2));
        assert_eq!(page.rows[1][1], Value::Text("o'neil".into()));
        assert!(page.query.ends_with("ORDER BY \"id\" LIMIT 2 OFFSET 1"));
        let names: Vec<_> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names[..3], ["id", "name", "age"]);

        // Value mapping by storage class; BOOLEAN columns show 0/1 as booleans.
        let first = d
            .browse_page(&req("people"), 0, 1)
            .await
            .unwrap()
            .rows
            .remove(0);
        assert_eq!(first[2], Value::Int(30));
        assert_eq!(first[3], Value::Bool(true));
        assert_eq!(first[4], Value::Float(1.5));
        assert_eq!(first[5], Value::bytes(vec![0xde, 0xad]));
        assert_eq!(first[6], Value::Text("2024-01-01 10:00:00".into()));
        // Zero dates are plain text.
        let third = d
            .browse_page(&req("people"), 2, 1)
            .await
            .unwrap()
            .rows
            .remove(0);
        assert_eq!(third[6], Value::Text("0000-00-00".into()));
        assert_eq!(third[3], Value::Null);

        // An empty page still reports columns.
        let empty = d.browse_page(&req("people"), 100, 10).await.unwrap();
        assert!(empty.rows.is_empty());
        assert_eq!(empty.columns.len(), 8);
        assert_eq!(empty.columns[3].family, TypeFamily::Boolean);
    }

    #[tokio::test]
    async fn filters_sorting_and_counts() {
        let d = test_driver().await;
        let mut r = req("people");
        r.filters = vec![Filter {
            column: "name".into(),
            op: FilterOp::Contains,
            value: Some("N".into()),
        }];
        assert_eq!(d.count_rows(&r).await.unwrap().exact, Some(2));
        r.sort = vec![Sort {
            column: "name".into(),
            descending: true,
        }];
        r.columns = vec!["name".into()];
        let page = d.browse_page(&r, 0, 10).await.unwrap();
        assert_eq!(
            page.rows,
            vec![
                vec![Value::Text("o'neil".into())],
                vec![Value::Text("ann".into())]
            ]
        );
        r.filters.clear();
        r.raw_where = Some("WHERE age IS NULL -- trailing comment".into());
        assert_eq!(d.count_rows(&r).await.unwrap().exact, Some(1));
        r.raw_where = Some("nonsense(".into());
        assert!(matches!(
            d.count_rows(&r).await,
            Err(Error::Database { .. })
        ));
    }

    #[tokio::test]
    async fn streams_results_with_columns_even_when_empty() {
        let d = test_driver().await;
        let events = collect(
            d.query(None, "SELECT id, name FROM people ORDER BY id")
                .await
                .unwrap(),
        )
        .await;
        assert!(matches!(&events[0], CursorEvent::Columns(c) if c.len() == 2 && c[0].name == "id"));
        let rows: usize = events
            .iter()
            .map(|e| {
                if let CursorEvent::Rows(r) = e {
                    r.len()
                } else {
                    0
                }
            })
            .sum();
        assert_eq!(rows, 3);

        let empty = collect(
            d.query(None, "SELECT id, name FROM people WHERE 0")
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(empty.len(), 1, "{empty:?}");
        assert!(matches!(&empty[0], CursorEvent::Columns(c) if c.len() == 2));

        let write = collect(
            d.query(None, "UPDATE people SET age = 31 WHERE id = 1")
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(write, vec![CursorEvent::Affected(1)]);

        // Expression columns get a family from their value.
        let expr = collect(
            d.query(None, "SELECT 1 AS one, 'x' AS s, 2.5 AS f")
                .await
                .unwrap(),
        )
        .await;
        let CursorEvent::Columns(c) = &expr[0] else {
            panic!("{expr:?}")
        };
        assert_eq!(
            c.iter().map(|c| c.family).collect::<Vec<_>>(),
            [TypeFamily::Integer, TypeFamily::Text, TypeFamily::Float]
        );
    }

    #[tokio::test]
    async fn grid_queries_truncate_but_exports_do_not() {
        let d = test_driver().await;
        let big = "x".repeat(40_000);
        let sql = format!("INSERT INTO no_pk VALUES ('{big}', 'z')");
        collect(d.query(None, &sql).await.unwrap()).await;

        let grid = collect(
            d.query(None, "SELECT a FROM no_pk WHERE b = 'z'")
                .await
                .unwrap(),
        )
        .await;
        let CursorEvent::Rows(rows) = &grid[1] else {
            panic!("{grid:?}")
        };
        assert!(matches!(
            rows[0][0],
            Value::TruncatedText { len: 40_000, .. }
        ));

        let mut r = req("no_pk");
        r.filters = vec![Filter {
            column: "b".into(),
            op: FilterOp::Eq,
            value: Some("z".into()),
        }];
        let all = collect(d.browse_all(&r).await.unwrap()).await;
        let CursorEvent::Rows(rows) = &all[1] else {
            panic!("{all:?}")
        };
        assert_eq!(rows[0][0], Value::Text(big));
    }

    #[tokio::test]
    async fn script_statements_share_a_connection() {
        let d = test_driver().await;
        for sql in ["BEGIN", "INSERT INTO no_pk VALUES ('tx', '1')"] {
            collect(d.query(None, sql).await.unwrap()).await;
        }
        // Same connection: the uncommitted row is visible; another connection would not see it.
        let seen = collect(
            d.query(None, "SELECT count(*) FROM no_pk WHERE a = 'tx'")
                .await
                .unwrap(),
        )
        .await;
        let CursorEvent::Rows(rows) = &seen[1] else {
            panic!("{seen:?}")
        };
        assert_eq!(rows[0][0], Value::Int(1));
        collect(d.query(None, "ROLLBACK").await.unwrap()).await;
    }

    #[tokio::test]
    async fn errors_carry_the_server_message() {
        let d = test_driver().await;
        let mut cursor = d.query(None, "SELEKT 1").await.unwrap();
        match cursor.next(10).await {
            Err(Error::Database { message, .. }) => {
                assert!(message.contains("syntax error"), "{message}")
            }
            other => panic!("{other:?}"),
        }
        // The connection is still usable afterwards.
        let events = collect(d.query(None, "SELECT 1").await.unwrap()).await;
        assert!(!events.is_empty());
    }

    #[tokio::test]
    async fn cancel_interrupts_a_running_statement() {
        let d = test_driver().await;
        // Endless recursive CTE with no rows until the end: only an interrupt stops it.
        let mut cursor = d
            .query(
                None,
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n",
            )
            .await
            .unwrap();
        let cancel = cursor.cancel_handle();
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            cancel.cancel().await.unwrap();
        });
        let result = tokio::time::timeout(Duration::from_secs(10), cursor.next(10)).await;
        canceller.await.unwrap();
        assert!(matches!(result, Ok(Err(Error::Cancelled))), "{result:?}");
        // Cancelling again, after the fact, is harmless.
        cursor.cancel_handle().cancel().await.unwrap();
        let events = collect(d.query(None, "SELECT 1").await.unwrap()).await;
        assert!(!events.is_empty());
    }

    #[tokio::test]
    async fn applies_changes_in_one_transaction() {
        let d = test_driver().await;
        let key = |id: i64| {
            vec![ColumnValue {
                column: "id".into(),
                value: Value::Int(id),
            }]
        };
        let text = |c: &str, v: &str| ColumnInput {
            column: c.into(),
            value: CellInput::Text(v.into()),
        };
        let changes = ChangeSet {
            table: table("people"),
            changes: vec![
                RowChange::Insert {
                    values: vec![text("name", "cy"), text("age", "44")],
                },
                RowChange::Update {
                    key: key(1),
                    original: vec![ColumnValue {
                        column: "name".into(),
                        value: Value::Text("ann".into()),
                    }],
                    set: vec![text("name", "anna")],
                },
                RowChange::Delete { key: key(2) },
            ],
        };
        let preview = d.preview_changes(&changes).await.unwrap();
        assert_eq!(preview.len(), 3);
        assert_eq!(
            preview[1],
            "UPDATE \"people\" SET \"name\" = 'anna' WHERE \"id\" = 1 AND \"name\" = 'ann'"
        );
        let applied = d.apply_changes(&changes).await.unwrap();
        assert_eq!(
            applied,
            ApplyResult {
                statements: 3,
                rows_affected: 3
            }
        );
        let page = d.browse_page(&req("people"), 0, 10).await.unwrap();
        let names: Vec<_> = page.rows.iter().map(|r| r[1].to_display()).collect();
        assert_eq!(names, ["anna", "o'neil", "cy"]);

        // The original value no longer matches: nothing is applied, even the valid first change.
        let stale = ChangeSet {
            table: table("people"),
            changes: vec![
                RowChange::Insert {
                    values: vec![text("name", "dee")],
                },
                RowChange::Update {
                    key: key(1),
                    original: vec![ColumnValue {
                        column: "name".into(),
                        value: Value::Text("ann".into()),
                    }],
                    set: vec![text("name", "zed")],
                },
            ],
        };
        match d.apply_changes(&stale).await {
            Err(Error::Database {
                message, statement, ..
            }) => {
                assert_eq!(message, "Row was changed or deleted by someone else");
                assert!(statement.unwrap().starts_with("UPDATE"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(d.count_rows(&req("people")).await.unwrap().exact, Some(3));

        // A constraint violation rolls back and names the statement.
        let dup = ChangeSet {
            table: table("people"),
            changes: vec![RowChange::Insert {
                values: vec![ColumnInput {
                    column: "id".into(),
                    value: CellInput::Text("1".into()),
                }],
            }],
        };
        assert!(matches!(
            d.apply_changes(&dup).await,
            Err(Error::Database {
                statement: Some(_),
                ..
            })
        ));

        // Read-only tables are refused.
        let ro = ChangeSet {
            table: table("no_pk"),
            changes: vec![RowChange::Delete { key: vec![] }],
        };
        assert!(matches!(
            d.apply_changes(&ro).await,
            Err(Error::InvalidInput(_))
        ));
    }

    #[tokio::test]
    async fn imports_rows_in_a_transaction() {
        let d = test_driver().await;
        let rows: Vec<Vec<Option<String>>> = (0..1100)
            .map(|i| {
                vec![
                    Some(format!("v{i}")),
                    if i % 2 == 0 {
                        None
                    } else {
                        Some("q'uote".into())
                    },
                ]
            })
            .collect();
        let n = d
            .insert_rows(&table("no_pk"), &["a".into(), "b".into()], rows)
            .await
            .unwrap();
        assert_eq!(n, 1100);
        assert_eq!(d.count_rows(&req("no_pk")).await.unwrap().exact, Some(1101));
        // A bad column rolls the whole import back.
        let err = d
            .insert_rows(
                &table("no_pk"),
                &["nope".into()],
                vec![vec![Some("1".into())]],
            )
            .await;
        assert!(err.is_err());
        assert_eq!(d.count_rows(&req("no_pk")).await.unwrap().exact, Some(1101));
    }

    #[tokio::test]
    async fn missing_file_is_a_connection_error() {
        let mut config = config_for("/definitely/not/here.db");
        let params = |c: &ConnectionConfig| ConnectParams {
            config: c.clone(),
            password: None,
            host: String::new(),
            port: 0,
        };
        assert!(matches!(
            connect(&params(&config)).await,
            Err(Error::Connection(_))
        ));
        config.file_path = None;
        assert!(matches!(
            connect(&params(&config)).await,
            Err(Error::InvalidInput(_))
        ));
    }

    // --- Live test against data/nasty.db (scripts/make-sqlite-db.sh); read-only -------------

    #[tokio::test]
    #[ignore = "needs data/nasty.db"]
    async fn live_nasty_db() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/nasty.db");
        let config = config_for(path);
        let d = connect(&ConnectParams {
            config,
            password: None,
            host: String::new(),
            port: 0,
        })
        .await
        .unwrap();

        let tables = d.list_tables("main", None).await.unwrap();
        let names: Vec<_> = tables.iter().map(|t| t.table.name.as_str()).collect();
        for expected in [
            "all_types",
            "big_table",
            "no_pk",
            "composite_pk",
            "select",
            "strict_table",
            "without_rowid",
            "docs_fts",
        ] {
            assert!(names.contains(&expected), "{expected} in {names:?}");
        }
        assert!(
            !names.iter().any(|n| n.starts_with("docs_fts_")),
            "{names:?}"
        );
        for info in &tables {
            let s = d
                .table_structure(&info.table)
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", info.table.name));
            let page = d
                .browse_page(&req(&info.table.name), 0, 25)
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", info.table.name));
            assert_eq!(page.columns.len(), s.columns.len(), "{}", info.table.name);
        }

        // Dynamic typing: the same column holds several storage classes.
        let page = d.browse_page(&req("all_types"), 0, 10).await.unwrap();
        let idx = |n: &str| page.columns.iter().position(|c| c.name == n).unwrap();
        assert_eq!(page.rows[0][idx("c_integer")], Value::Int(42));
        assert_eq!(
            page.rows[2][idx("c_integer")],
            Value::Text("not a number".into())
        );
        assert_eq!(page.rows[0][idx("c_boolean")], Value::Bool(true));
        assert_eq!(
            page.rows[0][idx("c_bigint")],
            Value::Int(9_007_199_254_740_993)
        );
        assert_eq!(page.rows[3][idx("c_integer")], Value::Int(i64::MAX));

        // 1M rows: first row fast, whole table streams.
        let started = Instant::now();
        let mut cursor = d.query(None, "SELECT * FROM big_table").await.unwrap();
        let first = cursor.next(100).await.unwrap();
        assert!(matches!(first, Some(CursorEvent::Columns(_))));
        let Some(CursorEvent::Rows(rows)) = cursor.next(100).await.unwrap() else {
            panic!()
        };
        assert_eq!(rows[0][0], Value::Int(1));
        eprintln!("first row of big_table: {:?}", started.elapsed());
        drop(cursor);
        let started = Instant::now();
        let count = d.count_rows(&req("big_table")).await.unwrap();
        assert_eq!(count.exact, Some(1_000_000));
        eprintln!("count of big_table: {:?}", started.elapsed());
        let started = Instant::now();
        let page = d
            .browse_page(&req("big_table"), 900_000, 100)
            .await
            .unwrap();
        assert_eq!(page.rows[0][0], Value::Int(900_001));
        eprintln!("page at offset 900000: {:?}", started.elapsed());
        let mut cursor = d.browse_all(&req("big_table")).await.unwrap();
        let mut total = 0;
        while let Some(e) = cursor.next(10_000).await.unwrap() {
            if let CursorEvent::Rows(r) = e {
                total += r.len();
            }
        }
        assert_eq!(total, 1_000_000);
    }
}
