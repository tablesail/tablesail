//! MySQL and MariaDB driver. Statements and browse queries run over the text
//! protocol (COM_QUERY), so every column arrives as bytes and is decoded by its
//! type name; zero dates and other oddities survive untouched.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use sqlx::mysql::{
    MySqlColumn, MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlRow, MySqlSslMode,
    MySqlTypeInfo,
};
use sqlx::pool::PoolConnection;
use sqlx::{
    AssertSqlSafe, Column, Connection, Either, Executor, MySql, Row, SqlSafeStr, Statement,
    TypeInfo, ValueRef,
};

use super::sqlcommon::{
    CancelState, ESTIMATE_ONLY_ABOVE, Idle, IdleList, Lease, Pump, StructureCache,
    apply_in_transaction, cancel_until_stopped, connection_lost, connection_survives, count_sql,
    editability, insert_in_transaction, insert_statements, is_unfiltered, meta, select_sql,
    tiebreak_columns,
};
use crate::Engine;
use crate::changes::change_statements;
use crate::cursor::ChannelCursor;
use crate::dialect::Dialect;
use crate::driver::{CancelHandle, Cursor, Driver};
use crate::error::{Error, Result};
use crate::model::{
    ApplyResult, BrowseRequest, ChangeSet, ColumnInfo, ColumnMeta, ConnectParams, ForeignKeyInfo,
    IndexInfo, Page, RowCount, SslMode, StatementKind, TableInfo, TableKind, TableRef,
    TableStructure, TypeFamily,
};
use crate::querylog::logged;
use crate::split::statement_kind;
use crate::value::{Row as DbRow, Value};

const MAX_CONNECTIONS: u32 = 8;
/// Databases hidden from the sidebar unless the connection's default database is one of them.
const SYSTEM_DATABASES: [&str; 4] = ["information_schema", "performance_schema", "mysql", "sys"];

pub async fn connect(params: &ConnectParams) -> Result<Arc<dyn Driver>> {
    let (options, default_db) = mysql_options(params)?;
    let pool = pool_options(MAX_CONNECTIONS).connect_with(options).await?;

    let mode = text_rows(&pool, "SELECT @@SESSION.sql_mode").await?;
    let backslash_escapes = !cell(&mode[0], 0).contains("NO_BACKSLASH_ESCAPES");
    let version = text_rows(&pool, "SELECT VERSION()").await?;
    let version = cell(&version[0], 0);
    let (version_label, mariadb) = version_label(&version);
    Ok(Arc::new(MysqlDriver {
        engine: params.config.engine,
        pool,
        dialect: Dialect::Mysql { backslash_escapes },
        mariadb,
        version: version_label,
        default_db,
        idle: Arc::new(IdleList::new(2)),
        cache: StructureCache::default(),
        control: None,
    }))
}

fn pool_options(max_connections: u32) -> MySqlPoolOptions {
    MySqlPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(15))
        .idle_timeout(Some(Duration::from_secs(600)))
        .after_connect(|conn, _| {
            Box::pin(async move {
                // Reading must not trip over zero dates; everything else stays as configured.
                let mode = fetch_text(&mut *conn, "SELECT @@SESSION.sql_mode").await?;
                let tolerant = tolerant_sql_mode(&mode);
                if tolerant != mode {
                    let set = format!(
                        "SET SESSION sql_mode = {}",
                        Dialect::Mysql {
                            backslash_escapes: !mode.contains("NO_BACKSLASH_ESCAPES")
                        }
                        .quote_string(&tolerant)
                    );
                    sqlx::raw_sql(logged(set)).execute(&mut *conn).await?;
                }
                Ok(())
            })
        })
}

/// "8.4.3-MySQL" / "11.8.2-MariaDB" from the raw `VERSION()` string, and whether it is MariaDB.
fn version_label(version: &str) -> (String, bool) {
    let mariadb = version.to_ascii_lowercase().contains("mariadb");
    // Old replication-compatible handshakes prefix MariaDB versions with "5.5.5-".
    let v = version.strip_prefix("5.5.5-").unwrap_or(version);
    let number = v.split('-').next().unwrap_or(v);
    let product = if mariadb { "MariaDB" } else { "MySQL" };
    (format!("{number}-{product}"), mariadb)
}

fn mysql_options(params: &ConnectParams) -> Result<(MySqlConnectOptions, Option<String>)> {
    let cfg = &params.config;
    let mut o = match cfg.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) => url
            .parse::<MySqlConnectOptions>()
            .map_err(|e| Error::InvalidInput(format!("Invalid connection URL: {e}")))?,
        None => MySqlConnectOptions::new(),
    };
    // With a URL the endpoint comes from it, unless an SSH tunnel moved it.
    let endpoint_moved = params.host != cfg.host || params.port != cfg.port;
    if cfg.url.is_none() || endpoint_moved {
        if !params.host.is_empty() {
            o = o.host(&params.host);
        } else if cfg.url.is_none() {
            o = o.host("localhost");
        }
        o = o.port(if params.port != 0 { params.port } else { 3306 });
    }
    if !cfg.user.is_empty() {
        o = o.username(&cfg.user);
    }
    if !cfg.database.is_empty() {
        o = o.database(&cfg.database);
    }
    if let Some(pw) = &params.password {
        o = o.password(pw);
    }

    o = o.ssl_mode(match cfg.ssl.mode {
        SslMode::Disable => MySqlSslMode::Disabled,
        SslMode::Prefer => MySqlSslMode::Preferred,
        SslMode::Require => MySqlSslMode::Required,
        SslMode::VerifyCa => MySqlSslMode::VerifyCa,
        SslMode::VerifyFull => MySqlSslMode::VerifyIdentity,
    });
    let path = |p: &Option<String>| {
        p.as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
    };
    if let Some(ca) = path(&cfg.ssl.ca_cert) {
        o = o.ssl_ca(ca);
    }
    if let Some(cert) = path(&cfg.ssl.client_cert) {
        o = o.ssl_client_cert(cert);
    }
    if let Some(key) = path(&cfg.ssl.client_key) {
        o = o.ssl_client_key(key);
    }

    o = o
        // Keep the server's own semantics for user queries: `||` stays OR, no sql_mode rewrites.
        .pipes_as_concat(false)
        .no_engine_substitution(false)
        // TIMESTAMPs show in the session time zone the server is configured with.
        .timezone(cfg.options.get("timezone").cloned())
        // Prepared statements only describe empty results and are closed straight away.
        .statement_cache_capacity(0);
    if let Some(charset) = cfg.options.get("charset") {
        o = o.charset(charset);
    }
    if let Some(collation) = cfg.options.get("collation") {
        o = o.collation(collation);
    }
    let default_db = o.get_database().map(str::to_owned);
    Ok((o, default_db))
}

/// `sql_mode` without NO_ZERO_DATE / NO_ZERO_IN_DATE.
fn tolerant_sql_mode(mode: &str) -> String {
    mode.split(',')
        .map(str::trim)
        .filter(|m| {
            !m.is_empty()
                && !m.eq_ignore_ascii_case("NO_ZERO_DATE")
                && !m.eq_ignore_ascii_case("NO_ZERO_IN_DATE")
        })
        .collect::<Vec<_>>()
        .join(",")
}

struct MysqlDriver {
    engine: Engine,
    pool: MySqlPool,
    dialect: Dialect,
    mariadb: bool,
    version: String,
    default_db: Option<String>,
    idle: Arc<IdleList<PoolConnection<MySql>>>,
    cache: StructureCache,
    /// Where cancels come from when `pool` has no connection to spare (a pinned driver's pool
    /// holds only the transaction's connection). Connects lazily, on the first cancel.
    control: Option<MySqlPool>,
}

type MysqlLease = Lease<PoolConnection<MySql>>;

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MyKind {
    /// `tinyint(1)`: a boolean while the value is 0 or 1.
    Bool,
    Int,
    UInt,
    Float,
    Decimal,
    Date,
    Time,
    DateTime,
    Json,
    Text,
    Bytes,
    Bit,
}

struct MyCol {
    meta: ColumnMeta,
    kind: MyKind,
}

/// A connection from the pool and its connection id.
async fn new_connection(pool: &MySqlPool) -> Result<(PoolConnection<MySql>, u64)> {
    let mut conn = pool.acquire().await?;
    let id = fetch_text(&mut conn, "SELECT CONNECTION_ID()")
        .await?
        .parse::<u64>()
        .map_err(|_| Error::Other("Could not read the connection id.".to_owned()))?;
    Ok((conn, id))
}

/// sqlx reports `_bin` collated text columns as binary because they carry the BINARY
/// flag; only collation 63 (`binary`) means raw bytes.
fn has_binary_collation(ti: &MySqlTypeInfo) -> bool {
    format!("{ti:?}").contains("Collation(63)")
}

/// Kind, family and display name for a column's sqlx type name.
fn classify(name: &str, binary_collation: bool) -> (MyKind, TypeFamily, String) {
    let text = |n: &str| (MyKind::Text, TypeFamily::Text, n.to_owned());
    let bytes = |n: &str| (MyKind::Bytes, TypeFamily::Binary, n.to_owned());
    match name {
        "BOOLEAN" => (MyKind::Bool, TypeFamily::Boolean, "BOOLEAN".to_owned()),
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "YEAR" => {
            (MyKind::Int, TypeFamily::Integer, name.to_owned())
        }
        n if n.ends_with(" UNSIGNED") => (MyKind::UInt, TypeFamily::Integer, name.to_owned()),
        "FLOAT" | "DOUBLE" => (MyKind::Float, TypeFamily::Float, name.to_owned()),
        "DECIMAL" => (MyKind::Decimal, TypeFamily::Decimal, name.to_owned()),
        "DATE" => (MyKind::Date, TypeFamily::Date, name.to_owned()),
        "TIME" => (MyKind::Time, TypeFamily::Time, name.to_owned()),
        "DATETIME" | "TIMESTAMP" => (MyKind::DateTime, TypeFamily::DateTime, name.to_owned()),
        "JSON" => (MyKind::Json, TypeFamily::Json, name.to_owned()),
        "ENUM" | "SET" => (MyKind::Text, TypeFamily::Enum, name.to_owned()),
        "GEOMETRY" => (MyKind::Bytes, TypeFamily::Spatial, name.to_owned()),
        "BIT" => (MyKind::Bit, TypeFamily::Other, name.to_owned()),
        // sqlx names string columns with the BINARY flag as the binary types.
        "BINARY" => {
            if binary_collation {
                bytes("BINARY")
            } else {
                text("CHAR")
            }
        }
        "VARBINARY" => {
            if binary_collation {
                bytes("VARBINARY")
            } else {
                text("VARCHAR")
            }
        }
        "TINYBLOB" => {
            if binary_collation {
                bytes("TINYBLOB")
            } else {
                text("TINYTEXT")
            }
        }
        "BLOB" => {
            if binary_collation {
                bytes("BLOB")
            } else {
                text("TEXT")
            }
        }
        "MEDIUMBLOB" => {
            if binary_collation {
                bytes("MEDIUMBLOB")
            } else {
                text("MEDIUMTEXT")
            }
        }
        "LONGBLOB" => {
            if binary_collation {
                bytes("LONGBLOB")
            } else {
                text("LONGTEXT")
            }
        }
        "CHAR" | "VARCHAR" | "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT" => text(name),
        _ => (MyKind::Text, TypeFamily::Other, name.to_owned()),
    }
}

/// sqlx names SET columns `CHAR`; the column flags tell them apart.
fn is_set_column(ti: &MySqlTypeInfo) -> bool {
    ti.name() == "CHAR" && format!("{ti:?}").contains("SET")
}

fn columns_of(columns: &[MySqlColumn]) -> Vec<MyCol> {
    columns
        .iter()
        .map(|c| {
            let ti = c.type_info();
            let (kind, family, name) = if is_set_column(ti) {
                classify("SET", false)
            } else {
                classify(ti.name(), has_binary_collation(ti))
            };
            MyCol {
                meta: meta(c.name(), name, family),
                kind,
            }
        })
        .collect()
}

fn decode_row(row: &MySqlRow, cols: &[MyCol], grid: bool) -> Result<DbRow> {
    let mut out = Vec::with_capacity(cols.len());
    for (i, col) in cols.iter().enumerate() {
        let v = if row.try_get_raw(i)?.is_null() {
            Value::Null
        } else {
            let bytes: &[u8] = row.try_get_unchecked(i)?;
            decode_bytes(bytes, col.kind, &col.meta.type_name)
        };
        out.push(if grid { v.for_grid() } else { v });
    }
    Ok(out)
}

/// Decodes the text-protocol form of one value; anything that does not parse stays text.
fn decode_bytes(b: &[u8], kind: MyKind, type_name: &str) -> Value {
    match kind {
        MyKind::Bytes => return Value::bytes(b.to_vec()),
        MyKind::Bit => {
            return Value::Other {
                type_name: type_name.to_owned(),
                text: bit_literal(b),
            };
        }
        _ => {}
    }
    let s = String::from_utf8_lossy(b);
    match kind {
        MyKind::Bool => match s.as_ref() {
            "0" => Value::Bool(false),
            "1" => Value::Bool(true),
            _ => s
                .parse::<i64>()
                .map(Value::Int)
                .unwrap_or_else(|_| Value::Text(s.into_owned())),
        },
        MyKind::Int => s
            .parse::<i64>()
            .map(Value::Int)
            .unwrap_or_else(|_| Value::Text(s.into_owned())),
        MyKind::UInt => match s.parse::<u64>() {
            Ok(u) => i64::try_from(u).map(Value::Int).unwrap_or(Value::UInt(u)),
            Err(_) => Value::Text(s.into_owned()),
        },
        MyKind::Float => s
            .parse::<f64>()
            .map(Value::Float)
            .unwrap_or_else(|_| Value::Text(s.into_owned())),
        MyKind::Decimal => Value::Decimal(s.into_owned()),
        // Zero dates (0000-00-00) and out-of-range times are kept exactly as sent.
        MyKind::Date => Value::Date(s.into_owned()),
        MyKind::Time => Value::Time(s.into_owned()),
        MyKind::DateTime => Value::DateTime(s.into_owned()),
        MyKind::Json => serde_json::from_str(&s)
            .map(Value::Json)
            .unwrap_or_else(|_| Value::Text(s.into_owned())),
        MyKind::Text => Value::Text(s.into_owned()),
        MyKind::Bytes | MyKind::Bit => unreachable!(),
    }
}

/// `b'1010'` from the big-endian bytes of a BIT value, without leading zeros.
fn bit_literal(b: &[u8]) -> String {
    let bits: String = b.iter().map(|byte| format!("{byte:08b}")).collect();
    let trimmed = bits.trim_start_matches('0');
    format!("b'{}'", if trimmed.is_empty() { "0" } else { trimmed })
}

// ---------------------------------------------------------------------------
// Catalog helpers
// ---------------------------------------------------------------------------

type TextRows = Vec<Vec<Option<String>>>;

fn rows_as_text(rows: &[MySqlRow]) -> Result<TextRows> {
    rows.iter()
        .map(|r| {
            (0..r.len())
                .map(|i| {
                    if r.try_get_raw(i)?.is_null() {
                        return Ok(None);
                    }
                    let b: &[u8] = r.try_get_unchecked(i)?;
                    Ok(Some(String::from_utf8_lossy(b).into_owned()))
                })
                .collect()
        })
        .collect()
}

/// Runs a catalog query over the text protocol; every cell comes back as text.
async fn text_rows(pool: &MySqlPool, sql: &str) -> Result<TextRows> {
    let rows = sqlx::raw_sql(logged(sql)).fetch_all(pool).await?;
    rows_as_text(&rows)
}

/// First cell of the first row, as text.
async fn fetch_text(
    conn: &mut sqlx::mysql::MySqlConnection,
    sql: &str,
) -> std::result::Result<String, sqlx::Error> {
    let row = sqlx::raw_sql(logged(sql)).fetch_one(&mut *conn).await?;
    let b: &[u8] = row.try_get_unchecked(0)?;
    Ok(String::from_utf8_lossy(b).into_owned())
}

fn cell(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

fn opt_cell(row: &[Option<String>], i: usize) -> Option<String> {
    row.get(i).cloned().flatten()
}

/// The values of `enum('a','b')` / `set('x','y')`; quotes are doubled inside values.
fn parse_enum_values(column_type: &str) -> Vec<String> {
    let Some(open) = column_type.find('(') else {
        return Vec::new();
    };
    let mut values = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut chars = column_type[open + 1..].chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if in_quote => {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    cur.push('\'');
                } else {
                    in_quote = false;
                    values.push(std::mem::take(&mut cur));
                }
            }
            '\'' => in_quote = true,
            c if in_quote => cur.push(c),
            _ => {}
        }
    }
    values
}

fn family_of_column(data_type: &str, column_type: &str) -> TypeFamily {
    match data_type.to_ascii_lowercase().as_str() {
        "tinyint" if column_type.to_ascii_lowercase().starts_with("tinyint(1)") => {
            TypeFamily::Boolean
        }
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "year" => {
            TypeFamily::Integer
        }
        "float" | "double" | "real" => TypeFamily::Float,
        "decimal" | "numeric" | "dec" | "fixed" => TypeFamily::Decimal,
        "char" | "varchar" | "tinytext" | "text" | "mediumtext" | "longtext" => TypeFamily::Text,
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => {
            TypeFamily::Binary
        }
        "enum" | "set" => TypeFamily::Enum,
        "json" => TypeFamily::Json,
        "date" => TypeFamily::Date,
        "time" => TypeFamily::Time,
        "datetime" | "timestamp" => TypeFamily::DateTime,
        "uuid" => TypeFamily::Uuid,
        "geometry" | "point" | "linestring" | "polygon" | "multipoint" | "multilinestring"
        | "multipolygon" | "geometrycollection" | "geomcollection" => TypeFamily::Spatial,
        _ => TypeFamily::Other,
    }
}

fn kind_of(table_type: &str) -> TableKind {
    match table_type {
        "VIEW" => TableKind::View,
        "SYSTEM VIEW" => TableKind::System,
        "SEQUENCE" => TableKind::Sequence,
        _ => TableKind::Table,
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

impl MysqlDriver {
    fn q(&self, s: &str) -> String {
        self.dialect.quote_string(s)
    }

    /// Fills in the connection's default database for an unqualified table.
    fn qualified_table(&self, table: &TableRef) -> Result<TableRef> {
        let database = if table.database.is_empty() {
            self.default_db
                .clone()
                .ok_or_else(|| Error::InvalidInput("Choose a database first.".to_owned()))?
        } else {
            table.database.clone()
        };
        Ok(TableRef {
            database,
            schema: None,
            name: table.name.clone(),
        })
    }

    async fn lease(&self) -> Result<MysqlLease> {
        // On a pinned driver this waits for the statement before; elsewhere it returns at once.
        let turn = self.idle.wait_turn().await?;
        while let Some(idle) = self.idle.take() {
            let stale = idle.is_stale();
            let mut conn = idle.conn;
            if stale && conn.ping().await.is_err() {
                continue;
            }
            return Ok(Lease::new(conn, idle.id, idle.db, self.idle.clone()).with_turn(turn));
        }
        if self.idle.is_pinned() {
            return Err(connection_lost());
        }
        let (conn, id) = new_connection(&self.pool).await?;
        // A fresh connection starts in the default database.
        Ok(Lease::new(
            conn,
            id,
            self.default_db.clone(),
            self.idle.clone(),
        ))
    }

    async fn start(
        &self,
        database: Option<&str>,
        sql: &str,
        grid: bool,
    ) -> Result<Box<dyn Cursor>> {
        let mut lease = self.lease().await?;
        // `lease.db` is the database last asked for on this connection. Switching only
        // when the request changes keeps a script's own `USE` in effect for its statements.
        let wanted = database
            .filter(|d| !d.is_empty())
            .map(str::to_owned)
            .or_else(|| self.default_db.clone());
        if let Some(db) = wanted
            && lease.db.as_deref() != Some(db.as_str())
        {
            let use_db = format!("USE {}", self.dialect.quote_ident(&db));
            let conn = &mut **lease.conn();
            match sqlx::raw_sql(logged(use_db)).execute(&mut *conn).await {
                Ok(_) => lease.db = Some(db),
                Err(e) => {
                    let err = Error::from(e);
                    if connection_survives(&err) {
                        lease.release();
                    } else {
                        lease.discard();
                    }
                    return Err(err);
                }
            }
        }
        let kind = statement_kind(sql, self.dialect);
        if kind == StatementKind::Ddl {
            // The cached table structures may be about to go stale.
            self.cache.clear();
        }
        let state = Arc::new(CancelState::default());
        state.start();
        let cancel = Arc::new(MysqlCancel {
            pool: self.control.as_ref().unwrap_or(&self.pool).clone(),
            id: lease.id,
            state: state.clone(),
        });
        lease.on_abandon({
            let cancel = cancel.clone();
            move || {
                Box::pin(async move {
                    // The connection is closed right after, so once is enough.
                    if cancel.state.request() {
                        let _ = cancel.fire().await;
                    }
                })
            }
        });
        let sql = sql.to_owned();
        let pinned = self.idle.is_pinned();
        let cursor = ChannelCursor::spawn(cancel, move |tx| {
            run_statement(
                lease,
                Pump::new(tx, grid).keep_reading(pinned),
                sql,
                kind,
                state,
            )
        });
        Ok(Box::new(if pinned {
            cursor.detach_on_drop()
        } else {
            cursor
        }))
    }

    async fn primary_key(&self, table: &TableRef) -> Result<Arc<Vec<String>>> {
        if let Some(k) = self.cache.primary_key(table) {
            return Ok(k);
        }
        let rows = text_rows(
            &self.pool,
            &format!(
                "SELECT COLUMN_NAME FROM information_schema.STATISTICS \
                 WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {} AND INDEX_NAME = 'PRIMARY' \
                 ORDER BY SEQ_IN_INDEX",
                self.q(&table.database),
                self.q(&table.name)
            ),
        )
        .await?;
        let key = rows.iter().map(|r| cell(r, 0)).collect();
        Ok(self.cache.put_primary_key(table, key))
    }

    async fn structure(&self, table: &TableRef) -> Result<Arc<TableStructure>> {
        let table = self.qualified_table(table)?;
        if let Some(s) = self.cache.structure(&table) {
            return Ok(s);
        }
        let s = self.read_structure(&table).await?;
        Ok(self.cache.put_structure(s))
    }

    async fn read_structure(&self, table: &TableRef) -> Result<TableStructure> {
        let (db, name) = (self.q(&table.database), self.q(&table.name));

        let rel = text_rows(
            &self.pool,
            &format!(
                "SELECT TABLE_TYPE FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = {db} AND TABLE_NAME = {name}"
            ),
        )
        .await?;
        let Some(rel) = rel.first() else {
            return Err(Error::NotFound(format!(
                "table {}.{}",
                table.database, table.name
            )));
        };
        let kind = kind_of(&cell(rel, 0));

        // name, column type, data type, nullable, default, key, extra, comment
        let col_rows = text_rows(
            &self.pool,
            &format!(
                "SELECT COLUMN_NAME, COLUMN_TYPE, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT, \
                        COLUMN_KEY, EXTRA, COLUMN_COMMENT \
                 FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = {db} AND TABLE_NAME = {name} ORDER BY ORDINAL_POSITION"
            ),
        )
        .await?;

        // name, non-unique, seq, column, type
        let stat_rows = text_rows(
            &self.pool,
            &format!(
                "SELECT INDEX_NAME, NON_UNIQUE, SEQ_IN_INDEX, COLUMN_NAME, INDEX_TYPE \
                 FROM information_schema.STATISTICS \
                 WHERE TABLE_SCHEMA = {db} AND TABLE_NAME = {name} \
                 ORDER BY INDEX_NAME, SEQ_IN_INDEX"
            ),
        )
        .await?;
        let mut indexes: Vec<IndexInfo> = Vec::new();
        for r in &stat_rows {
            let index_name = cell(r, 0);
            // Functional index parts have no column name.
            let column = opt_cell(r, 3).unwrap_or_else(|| "<expression>".to_owned());
            match indexes.iter_mut().find(|i| i.name == index_name) {
                Some(i) => i.columns.push(column),
                None => indexes.push(IndexInfo {
                    primary: index_name == "PRIMARY",
                    unique: cell(r, 1) == "0",
                    method: opt_cell(r, 4),
                    columns: vec![column],
                    name: index_name,
                }),
            }
        }
        indexes.sort_by_key(|i| !i.primary);
        let primary_key = indexes
            .iter()
            .find(|i| i.primary)
            .map(|i| i.columns.clone())
            .unwrap_or_default();

        // constraint, column, position, ref schema, ref table, ref column, update rule, delete rule
        let fk_rows = text_rows(
            &self.pool,
            &format!(
                "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.ORDINAL_POSITION, \
                        k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, \
                        r.UPDATE_RULE, r.DELETE_RULE \
                 FROM information_schema.KEY_COLUMN_USAGE k \
                 JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
                   ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA \
                  AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME \
                 WHERE k.TABLE_SCHEMA = {db} AND k.TABLE_NAME = {name} \
                   AND k.REFERENCED_TABLE_NAME IS NOT NULL \
                 ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION"
            ),
        )
        .await?;
        let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
        for r in &fk_rows {
            let fk_name = cell(r, 0);
            match foreign_keys.iter_mut().find(|f| f.name == fk_name) {
                Some(f) => {
                    f.columns.push(cell(r, 1));
                    f.referenced_columns.push(cell(r, 5));
                }
                None => foreign_keys.push(ForeignKeyInfo {
                    name: fk_name,
                    columns: vec![cell(r, 1)],
                    referenced: TableRef {
                        database: cell(r, 3),
                        schema: None,
                        name: cell(r, 4),
                    },
                    referenced_columns: vec![cell(r, 5)],
                    on_update: opt_cell(r, 6),
                    on_delete: opt_cell(r, 7),
                }),
            }
        }

        let columns: Vec<ColumnInfo> = col_rows
            .iter()
            .map(|r| {
                let column_type = cell(r, 1);
                let family = family_of_column(&cell(r, 2), &column_type);
                let extra = cell(r, 6).to_ascii_lowercase();
                let name = cell(r, 0);
                let mut default = opt_cell(r, 4);
                // MariaDB spells "no default" as the text NULL.
                if self.mariadb && default.as_deref() == Some("NULL") {
                    default = None;
                }
                ColumnInfo {
                    is_primary_key: primary_key.contains(&name),
                    nullable: cell(r, 3) == "YES",
                    default,
                    is_auto_increment: extra.contains("auto_increment"),
                    is_generated: extra.contains("generated")
                        && !extra.contains("default_generated"),
                    enum_values: matches!(cell(r, 2).to_ascii_lowercase().as_str(), "enum" | "set")
                        .then(|| parse_enum_values(&column_type)),
                    comment: opt_cell(r, 7).filter(|c| !c.is_empty()),
                    data_type: column_type,
                    family,
                    name,
                }
            })
            .collect();

        let ddl = text_rows(
            &self.pool,
            &format!(
                "SHOW CREATE {} {}.{}",
                if kind == TableKind::View {
                    "VIEW"
                } else {
                    "TABLE"
                },
                self.dialect.quote_ident(&table.database),
                self.dialect.quote_ident(&table.name)
            ),
        )
        .await
        .ok()
        .and_then(|rows| rows.first().and_then(|r| opt_cell(r, 1)));

        Ok(TableStructure {
            table: table.clone(),
            kind,
            editability: editability(kind, &primary_key),
            columns,
            primary_key,
            indexes,
            foreign_keys,
            ddl,
        })
    }

    async fn estimate_rows(&self, table: &TableRef) -> Result<Option<u64>> {
        let rows = text_rows(
            &self.pool,
            &format!(
                "SELECT TABLE_ROWS FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {} AND TABLE_TYPE = 'BASE TABLE'",
                self.q(&table.database),
                self.q(&table.name)
            ),
        )
        .await?;
        Ok(rows.first().and_then(|r| cell(r, 0).parse().ok()))
    }
}

/// Cancels the running statement with `KILL QUERY` from a second connection.
struct MysqlCancel {
    pool: MySqlPool,
    id: u64,
    state: Arc<CancelState>,
}

#[async_trait]
impl CancelHandle for MysqlCancel {
    async fn cancel(&self) -> Result<()> {
        if !self.state.request() {
            return Ok(());
        }
        cancel_until_stopped(&self.state, || self.fire()).await
    }
}

impl MysqlCancel {
    /// One `KILL QUERY`.
    async fn fire(&self) -> Result<()> {
        sqlx::raw_sql(logged(format!("KILL QUERY {}", self.id)))
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Driver for MysqlDriver {
    fn engine(&self) -> Engine {
        self.engine
    }

    fn dialect(&self) -> Dialect {
        self.dialect
    }

    async fn server_version(&self) -> Result<String> {
        Ok(self.version.clone())
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        let rows = text_rows(&self.pool, "SHOW DATABASES").await?;
        Ok(rows
            .iter()
            .map(|r| cell(r, 0))
            .filter(|d| {
                self.default_db.as_deref() == Some(d.as_str())
                    || !SYSTEM_DATABASES.contains(&d.to_ascii_lowercase().as_str())
            })
            .collect())
    }

    async fn list_schemas(&self, _database: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn list_tables(&self, database: &str, _schema: Option<&str>) -> Result<Vec<TableInfo>> {
        let rows = text_rows(
            &self.pool,
            &format!(
                "SELECT TABLE_NAME, TABLE_TYPE, TABLE_ROWS, TABLE_COMMENT \
                 FROM information_schema.TABLES WHERE TABLE_SCHEMA = {} ORDER BY TABLE_NAME",
                self.q(database)
            ),
        )
        .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let table_type = cell(r, 1);
                let kind = kind_of(&table_type);
                TableInfo {
                    table: TableRef {
                        database: database.to_owned(),
                        schema: None,
                        name: cell(r, 0),
                    },
                    kind,
                    estimated_rows: if kind == TableKind::Table {
                        cell(r, 2).parse().ok()
                    } else {
                        None
                    },
                    comment: opt_cell(r, 3).filter(|c| !c.is_empty() && c != "VIEW"),
                }
            })
            .collect())
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        let table = self.qualified_table(table)?;
        let s = self.read_structure(&table).await?;
        Ok((*self.cache.put_structure(s)).clone())
    }

    async fn browse_page(&self, request: &BrowseRequest, offset: u64, limit: u32) -> Result<Page> {
        let started = Instant::now();
        let mut request = request.clone();
        request.table = self.qualified_table(&request.table)?;
        // The key orders unsorted pages and breaks ties between sorted ones; a table
        // without one is tie-broken by its columns.
        let mut key = self.primary_key(&request.table).await?;
        if key.is_empty() && !request.sort.is_empty() {
            key = Arc::new(tiebreak_columns(&*self.structure(&request.table).await?));
        }
        let sql = select_sql(self.dialect, &request, &key, Some((limit, offset)))?;
        let rows = sqlx::raw_sql(logged(sql.clone()))
            .fetch_all(&self.pool)
            .await?;
        let cols = match rows.first() {
            Some(first) => columns_of(first.columns()),
            None => {
                let stmt = (&self.pool)
                    .prepare(AssertSqlSafe(sql.clone()).into_sql_str())
                    .await?;
                columns_of(stmt.columns())
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
        let mut request = request.clone();
        request.table = self.qualified_table(&request.table)?;
        let estimate = if is_unfiltered(&request) {
            self.estimate_rows(&request.table).await.unwrap_or(None)
        } else {
            None
        };
        if estimate.is_some_and(|n| n > ESTIMATE_ONLY_ABOVE) {
            return Ok(RowCount {
                exact: None,
                estimate,
            });
        }
        let sql = count_sql(self.dialect, &request)?;
        let rows = text_rows(&self.pool, &sql).await?;
        let exact = cell(&rows[0], 0).parse::<u64>().ok();
        Ok(RowCount { exact, estimate })
    }

    async fn browse_all(&self, request: &BrowseRequest) -> Result<Box<dyn Cursor>> {
        let mut request = request.clone();
        request.table = self.qualified_table(&request.table)?;
        let sql = select_sql(self.dialect, &request, &[], None)?;
        self.start(None, &sql, false).await
    }

    async fn query(&self, database: Option<&str>, sql: &str) -> Result<Box<dyn Cursor>> {
        self.start(database, sql, true).await
    }

    async fn preview_changes(&self, changes: &ChangeSet) -> Result<Vec<String>> {
        let mut changes = changes.clone();
        changes.table = self.qualified_table(&changes.table)?;
        let structure = self.structure(&changes.table).await?;
        change_statements(self.dialect, &structure, &changes)
    }

    async fn apply_changes(&self, changes: &ChangeSet) -> Result<ApplyResult> {
        let mut changes = changes.clone();
        changes.table = self.qualified_table(&changes.table)?;
        let structure = self.structure(&changes.table).await?;
        let statements = change_statements(self.dialect, &structure, &changes)?;
        apply_in_transaction(&self.pool, &statements, &changes.changes).await
    }

    async fn insert_rows(
        &self,
        table: &TableRef,
        columns: &[String],
        rows: Vec<Vec<Option<String>>>,
    ) -> Result<u64> {
        let table = self.qualified_table(table)?;
        let statements = insert_statements(self.dialect, &table, columns, &rows)?;
        insert_in_transaction(&self.pool, &statements).await
    }

    async fn pinned(&self, database: Option<&str>) -> Result<Arc<dyn Driver>> {
        let database = database.filter(|d| !d.is_empty()).map(str::to_owned);
        let mut options = (*self.pool.connect_options()).clone();
        if let Some(db) = &database {
            options = options.database(db);
        }
        let control = pool_options(1)
            .idle_timeout(Some(Duration::from_secs(30)))
            .connect_lazy_with(options.clone());
        let pool = pool_options(1)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(options)
            .await?;
        let default_db = database.or_else(|| self.default_db.clone());
        let (conn, id) = new_connection(&pool).await?;
        let idle = IdleList::pinned();
        idle.give(Idle {
            conn,
            id,
            db: default_db.clone(),
            since: Instant::now(),
        });
        Ok(Arc::new(MysqlDriver {
            engine: self.engine,
            pool,
            dialect: self.dialect,
            mariadb: self.mariadb,
            version: self.version.clone(),
            default_db,
            idle: Arc::new(idle),
            cache: StructureCache::default(),
            control: Some(control),
        }))
    }

    fn forget_structures(&self) {
        self.cache.clear();
    }

    async fn close(&self) {
        self.cache.clear();
        self.idle.close();
        self.pool.close().await;
        if let Some(control) = &self.control {
            control.close().await;
        }
    }
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

async fn run_statement(
    mut lease: MysqlLease,
    mut pump: Pump,
    sql: String,
    kind: StatementKind,
    state: Arc<CancelState>,
) -> Result<()> {
    let outcome = stream_statement(&mut lease, &sql, kind, &mut pump).await;
    state.finish();
    match outcome {
        Ok(()) => {
            lease.release();
            Ok(())
        }
        Err(err) => {
            if connection_survives(&err) {
                lease.release();
            } else {
                lease.discard();
            }
            Err(state.map_error(err))
        }
    }
}

async fn stream_statement(
    lease: &mut MysqlLease,
    sql: &str,
    kind: StatementKind,
    pump: &mut Pump,
) -> Result<()> {
    let conn = &mut **lease.conn();
    let mut cols: Vec<MyCol> = Vec::new();
    let mut deferred: Option<u64> = None;
    {
        let mut stream = sqlx::raw_sql(logged(sql)).fetch_many(&mut *conn);
        while let Some(item) = stream.next().await {
            match item? {
                Either::Right(_) if pump.gone() => {}
                Either::Right(row) => {
                    if !pump.in_set() {
                        cols = columns_of(row.columns());
                        pump.columns(cols.iter().map(|c| c.meta.clone()).collect())
                            .await?;
                    }
                    pump.row(decode_row(&row, &cols, pump.grid())?).await?;
                }
                // Ends a result set (rows were sent) or reports a statement without rows.
                Either::Left(result) => {
                    if let Some(n) = pump
                        .end_set(result.rows_affected(), kind == StatementKind::Read)
                        .await?
                    {
                        deferred = Some(n);
                    }
                }
            }
        }
    }
    if let Some(affected) = deferred
        && !pump.gone()
    {
        // A query that returned no rows still has columns; prepare it to read them.
        match (&mut *conn)
            .prepare(AssertSqlSafe(sql.to_owned()).into_sql_str())
            .await
        {
            Ok(stmt) if !stmt.columns().is_empty() => {
                let cols = columns_of(stmt.columns());
                pump.columns(cols.into_iter().map(|c| c.meta).collect())
                    .await?;
                pump.end_set(0, false).await?;
            }
            _ => pump.affected(affected).await?,
        }
    }
    pump.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionConfig, EnvTag, SslConfig};
    use std::collections::BTreeMap;

    fn config() -> ConnectionConfig {
        ConnectionConfig {
            id: String::new(),
            name: "my".into(),
            engine: Engine::Mysql,
            host: "db.example.com".into(),
            port: 3307,
            user: "alice".into(),
            database: "shop".into(),
            file_path: None,
            url: None,
            ssl: SslConfig::default(),
            ssh: None,
            env: EnvTag::Local,
            color: None,
            read_only: false,
            options: BTreeMap::new(),
        }
    }

    fn params(config: ConnectionConfig) -> ConnectParams {
        ConnectParams {
            host: config.host.clone(),
            port: config.port,
            password: Some("s3cret".into()),
            config,
        }
    }

    #[test]
    fn options_from_fields() {
        let (o, db) = mysql_options(&params(config())).unwrap();
        assert_eq!(o.get_host(), "db.example.com");
        assert_eq!(o.get_port(), 3307);
        assert_eq!(o.get_username(), "alice");
        assert_eq!(o.get_database(), Some("shop"));
        assert_eq!(db.as_deref(), Some("shop"));
    }

    #[test]
    fn tunnel_and_url() {
        let mut p = params(config());
        p.host = "127.0.0.1".into();
        p.port = 41000;
        let (o, _) = mysql_options(&p).unwrap();
        assert_eq!((o.get_host(), o.get_port()), ("127.0.0.1", 41000));

        let mut c = config();
        c.host = String::new();
        c.port = 0;
        c.user = String::new();
        c.database = String::new();
        c.url = Some("mysql://bob@my.internal:3399/orders".into());
        let (o, db) = mysql_options(&params(c.clone())).unwrap();
        assert_eq!(
            (o.get_host(), o.get_port(), o.get_username()),
            ("my.internal", 3399, "bob")
        );
        assert_eq!(db.as_deref(), Some("orders"));
        c.url = Some("::bad::".into());
        assert!(matches!(
            mysql_options(&params(c)),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn zero_date_modes_are_removed() {
        assert_eq!(
            tolerant_sql_mode(
                "ONLY_FULL_GROUP_BY,STRICT_TRANS_TABLES,NO_ZERO_IN_DATE,NO_ZERO_DATE,ERROR_FOR_DIVISION_BY_ZERO"
            ),
            "ONLY_FULL_GROUP_BY,STRICT_TRANS_TABLES,ERROR_FOR_DIVISION_BY_ZERO"
        );
        assert_eq!(tolerant_sql_mode(""), "");
        assert_eq!(tolerant_sql_mode("NO_ZERO_DATE"), "");
    }

    #[test]
    fn classifies_sqlx_type_names() {
        assert_eq!(classify("BOOLEAN", false).0, MyKind::Bool);
        assert_eq!(classify("BIGINT UNSIGNED", false).0, MyKind::UInt);
        assert_eq!(classify("YEAR", false).0, MyKind::Int);
        assert_eq!(classify("DECIMAL", false).1, TypeFamily::Decimal);
        assert_eq!(classify("ENUM", false).1, TypeFamily::Enum);
        assert_eq!(classify("SET", false).1, TypeFamily::Enum);
        assert_eq!(classify("BIT", false).0, MyKind::Bit);
        assert_eq!(classify("GEOMETRY", false).1, TypeFamily::Spatial);
        // Real binary columns vs `_bin` collated text (both carry the BINARY flag).
        assert_eq!(classify("VARBINARY", true).0, MyKind::Bytes);
        assert_eq!(
            classify("VARBINARY", false),
            (MyKind::Text, TypeFamily::Text, "VARCHAR".into())
        );
        assert_eq!(classify("BLOB", false).2, "TEXT");
        assert_eq!(classify("LONGBLOB", true).1, TypeFamily::Binary);
    }

    #[test]
    fn decodes_text_protocol_values() {
        use MyKind::*;
        assert_eq!(decode_bytes(b"1", Bool, "BOOLEAN"), Value::Bool(true));
        assert_eq!(decode_bytes(b"0", Bool, "BOOLEAN"), Value::Bool(false));
        // tinyint(1) holding something else stays a number.
        assert_eq!(decode_bytes(b"5", Bool, "BOOLEAN"), Value::Int(5));
        assert_eq!(decode_bytes(b"-128", Int, "TINYINT"), Value::Int(-128));
        assert_eq!(decode_bytes(b"42", UInt, "INT UNSIGNED"), Value::Int(42));
        assert_eq!(
            decode_bytes(b"18446744073709551615", UInt, "BIGINT UNSIGNED"),
            Value::UInt(u64::MAX)
        );
        assert_eq!(decode_bytes(b"1.5", Float, "DOUBLE"), Value::Float(1.5));
        assert_eq!(
            decode_bytes(b"12.3400", Decimal, "DECIMAL"),
            Value::Decimal("12.3400".into())
        );
        // Zero dates and extreme times survive as sent.
        assert_eq!(
            decode_bytes(b"0000-00-00", Date, "DATE"),
            Value::Date("0000-00-00".into())
        );
        assert_eq!(
            decode_bytes(b"0000-00-00 00:00:00", DateTime, "DATETIME"),
            Value::DateTime("0000-00-00 00:00:00".into())
        );
        assert_eq!(
            decode_bytes(b"-838:59:59", Time, "TIME"),
            Value::Time("-838:59:59".into())
        );
        assert_eq!(decode_bytes(b"2024", Int, "YEAR"), Value::Int(2024));
        assert_eq!(
            decode_bytes(br#"{"a": [1]}"#, Json, "JSON"),
            Value::Json(serde_json::json!({"a": [1]}))
        );
        assert_eq!(
            decode_bytes("héllo".as_bytes(), Text, "VARCHAR"),
            Value::Text("héllo".into())
        );
        assert_eq!(
            decode_bytes(&[0, 255, 1], Bytes, "BLOB"),
            Value::bytes(vec![0, 255, 1])
        );
        assert_eq!(
            decode_bytes(&[0b0000_0101], Bit, "BIT"),
            Value::Other {
                type_name: "BIT".into(),
                text: "b'101'".into()
            }
        );
        assert_eq!(bit_literal(&[0, 0]), "b'0'");
        assert_eq!(bit_literal(&[1, 0]), "b'100000000'");
    }

    #[test]
    fn labels_versions() {
        assert_eq!(version_label("8.4.3"), ("8.4.3-MySQL".to_owned(), false));
        assert_eq!(
            version_label("8.0.36-0ubuntu0.22.04.1"),
            ("8.0.36-MySQL".to_owned(), false)
        );
        assert_eq!(
            version_label("11.8.2-MariaDB"),
            ("11.8.2-MariaDB".to_owned(), true)
        );
        assert_eq!(
            version_label("5.5.5-10.11.6-MariaDB-1:10.11.6+maria~deb12"),
            ("10.11.6-MariaDB".to_owned(), true)
        );
    }

    #[test]
    fn parses_enum_definitions() {
        assert_eq!(
            parse_enum_values("enum('a','b c','it''s')"),
            ["a", "b c", "it's"]
        );
        assert_eq!(parse_enum_values("set('x','y')"), ["x", "y"]);
        assert_eq!(parse_enum_values("varchar(255)"), Vec::<String>::new());
        assert_eq!(parse_enum_values("enum('a,b','')"), ["a,b", ""]);
    }

    #[test]
    fn column_families() {
        assert_eq!(
            family_of_column("tinyint", "tinyint(1)"),
            TypeFamily::Boolean
        );
        assert_eq!(
            family_of_column("tinyint", "tinyint(4)"),
            TypeFamily::Integer
        );
        assert_eq!(family_of_column("varchar", "varchar(20)"), TypeFamily::Text);
        assert_eq!(
            family_of_column("varbinary", "varbinary(20)"),
            TypeFamily::Binary
        );
        assert_eq!(
            family_of_column("multipolygon", "multipolygon"),
            TypeFamily::Spatial
        );
        assert_eq!(family_of_column("bit", "bit(8)"), TypeFamily::Other);
        assert_eq!(kind_of("VIEW"), TableKind::View);
        assert_eq!(kind_of("BASE TABLE"), TableKind::Table);
    }

    // --- Live tests against the docker "nasty schema" (cargo test -- --ignored) -------------

    use crate::driver::CursorEvent;
    use crate::model::{ColumnInput, ColumnValue, Editability, Filter, FilterOp, RowChange, Sort};
    use crate::value::CellInput;

    async fn live(engine: Engine, port: u16, env: &str) -> Arc<dyn Driver> {
        let mut c = config();
        c.engine = engine;
        c.host = "127.0.0.1".into();
        c.port = port;
        c.user = "tablesail".into();
        c.database = "tablesail".into();
        if let Ok(url) = std::env::var(env) {
            c.host = String::new();
            c.port = 0;
            c.user = String::new();
            c.database = String::new();
            c.url = Some(url);
        }
        let mut p = params(c);
        p.password = Some("tablesail".into());
        connect(&p)
            .await
            .expect("the docker test database is not running")
    }

    fn t(db: &str, name: &str) -> TableRef {
        TableRef {
            database: db.into(),
            schema: None,
            name: name.into(),
        }
    }

    fn browse(table: TableRef) -> BrowseRequest {
        BrowseRequest {
            table,
            filters: vec![],
            match_all: true,
            raw_where: None,
            sort: vec![],
            columns: vec![],
        }
    }

    async fn drain(mut cursor: Box<dyn Cursor>) -> Result<Vec<CursorEvent>> {
        let mut out = Vec::new();
        while let Some(e) = cursor.next(5000).await? {
            out.push(e);
        }
        Ok(out)
    }

    async fn suite_catalog(d: Arc<dyn Driver>, mariadb: bool) {
        let version = d.server_version().await.unwrap();
        assert!(
            version.ends_with(if mariadb { "-MariaDB" } else { "-MySQL" }),
            "{version}"
        );
        let dbs = d.list_databases().await.unwrap();
        assert!(
            dbs.contains(&"tablesail".to_owned()) && dbs.contains(&"tablesail_other".to_owned()),
            "{dbs:?}"
        );
        assert!(!dbs.contains(&"information_schema".to_owned()), "{dbs:?}");
        assert!(d.list_schemas("tablesail").await.unwrap().is_empty());

        let tables = d.list_tables("tablesail", None).await.unwrap();
        let find = |n: &str| {
            tables
                .iter()
                .find(|t| t.table.name == n)
                .unwrap_or_else(|| panic!("missing {n}"))
        };
        assert_eq!(find("customer_totals").kind, TableKind::View);
        assert_eq!(find("big_table").kind, TableKind::Table);
        assert!(find("big_table").estimated_rows.unwrap() > 800_000);
        assert!(
            tables
                .iter()
                .any(|t| t.table.name == "weird \"quoted\" name; DROP TABLE x;--")
        );

        for info in &tables {
            let name = &info.table.name;
            let s = d
                .table_structure(&t("tablesail", name))
                .await
                .unwrap_or_else(|e| panic!("structure of {name}: {e}"));
            assert!(!s.columns.is_empty(), "{name}");
            assert!(s.ddl.is_some(), "ddl of {name}");
            let page = d
                .browse_page(&browse(t("tablesail", name)), 0, 25)
                .await
                .unwrap_or_else(|e| panic!("browse {name}: {e}"));
            assert_eq!(page.columns.len(), s.columns.len(), "{name}");
            d.count_rows(&browse(t("tablesail", name)))
                .await
                .unwrap_or_else(|e| panic!("count {name}: {e}"));
        }

        let orders = d.table_structure(&t("tablesail", "orders")).await.unwrap();
        assert_eq!(orders.primary_key, ["id"]);
        assert_eq!(orders.editability, Editability::Editable);
        let col = |n: &str| orders.columns.iter().find(|c| c.name == n).unwrap();
        assert!(col("id").is_auto_increment);
        assert_eq!(col("status").family, TypeFamily::Enum);
        assert_eq!(
            col("status").enum_values.as_deref(),
            Some(&["pending".to_owned(), "paid".into(), "shipped".into()][..])
        );
        assert_eq!(col("total").data_type, "decimal(12,2)");
        assert!(!col("total").nullable);
        assert_eq!(orders.foreign_keys.len(), 1);
        assert_eq!(orders.foreign_keys[0].name, "orders_customer_fk");
        assert_eq!(orders.foreign_keys[0].referenced.database, "tablesail");
        assert_eq!(orders.foreign_keys[0].referenced.name, "customers");
        assert_eq!(orders.foreign_keys[0].on_delete.as_deref(), Some("CASCADE"));
        assert!(
            orders
                .indexes
                .iter()
                .any(|i| i.primary && i.columns == ["id"])
        );
        assert!(orders.ddl.unwrap().starts_with("CREATE TABLE `orders`"));

        let computed = d
            .table_structure(&t("tablesail", "computed"))
            .await
            .unwrap();
        assert!(
            computed
                .columns
                .iter()
                .find(|c| c.name == "total")
                .unwrap()
                .is_generated
        );
        assert!(
            computed
                .columns
                .iter()
                .find(|c| c.name == "label")
                .unwrap()
                .is_generated
        );
        let all = d
            .table_structure(&t("tablesail", "all_types"))
            .await
            .unwrap();
        let fam = |n: &str| all.columns.iter().find(|c| c.name == n).unwrap().family;
        assert_eq!(fam("c_bool"), TypeFamily::Boolean);
        assert_eq!(fam("c_set"), TypeFamily::Enum);
        assert_eq!(fam("c_point"), TypeFamily::Spatial);
        assert_eq!(
            all.columns
                .iter()
                .find(|c| c.name == "c_set")
                .unwrap()
                .enum_values
                .as_deref(),
            Some(&["a".to_owned(), "b".into(), "c".into()][..])
        );
        let no_pk = d.table_structure(&t("tablesail", "no_pk")).await.unwrap();
        assert!(matches!(no_pk.editability, Editability::ReadOnly(_)));
        let view = d
            .table_structure(&t("tablesail", "customer_totals"))
            .await
            .unwrap();
        assert_eq!(
            view.editability,
            Editability::ReadOnly("Views can't be edited.".into())
        );
        assert_eq!(
            d.table_structure(&t("tablesail", "composite_pk"))
                .await
                .unwrap()
                .primary_key,
            ["tenant_id", "item_id"]
        );
        assert!(matches!(
            d.table_structure(&t("tablesail", "nope")).await,
            Err(Error::NotFound(_))
        ));

        let customers = d
            .count_rows(&browse(t("tablesail", "customers")))
            .await
            .unwrap();
        assert_eq!(customers.exact, Some(2));
        let big = d
            .count_rows(&browse(t("tablesail", "big_table")))
            .await
            .unwrap();
        assert!(
            big.exact.is_none() || big.exact == Some(1_000_000),
            "{big:?}"
        );
        let mut filtered = browse(t("tablesail", "big_table"));
        filtered.filters = vec![Filter {
            column: "name".into(),
            op: FilterOp::StartsWith,
            value: Some("row 99999".into()),
        }];
        assert_eq!(d.count_rows(&filtered).await.unwrap().exact, Some(11));
        let started = Instant::now();
        let page = d
            .browse_page(&browse(t("tablesail", "big_table")), 500_000, 100)
            .await
            .unwrap();
        assert_eq!(page.rows[0][0], Value::Int(500_001));
        eprintln!("big_table page at offset 500000: {:?}", started.elapsed());
        let mut sorted = browse(t("tablesail", "big_table"));
        sorted.sort = vec![Sort {
            column: "id".into(),
            descending: true,
        }];
        assert_eq!(
            d.browse_page(&sorted, 0, 3).await.unwrap().rows[0][0],
            Value::Int(1_000_000)
        );
    }

    async fn suite_values(d: Arc<dyn Driver>, mariadb: bool) {
        let mut r = browse(t("tablesail", "all_types"));
        r.sort = vec![Sort {
            column: "id".into(),
            descending: false,
        }];
        let page = d.browse_page(&r, 0, 10).await.unwrap();
        assert_eq!(page.rows.len(), 4);
        let idx = |n: &str| {
            page.columns
                .iter()
                .position(|c| c.name == n)
                .unwrap_or_else(|| panic!("{n}"))
        };
        let get = |row: usize, n: &str| page.rows[row][idx(n)].clone();
        assert_eq!(get(0, "id"), Value::Int(1));
        assert_eq!(get(0, "c_bool"), Value::Bool(true));
        assert_eq!(
            get(3, "c_bool"),
            Value::Int(5),
            "tinyint(1) holding 5 is not a boolean"
        );
        assert_eq!(get(2, "c_bool"), Value::Bool(false));
        assert_eq!(get(0, "c_tinyint_unsigned"), Value::Int(200));
        assert_eq!(get(0, "c_bigint"), Value::Int(9_007_199_254_740_993));
        assert_eq!(get(0, "c_bigint_unsigned"), Value::UInt(u64::MAX));
        assert_eq!(get(2, "c_bigint"), Value::Int(i64::MIN));
        assert!(
            matches!(get(0, "c_decimal"), Value::Decimal(ref s) if s.starts_with("12345.6789012345"))
        );
        assert_eq!(get(0, "c_float"), Value::Float(1.5));
        assert_eq!(get(0, "c_double"), Value::Float(std::f64::consts::E));
        assert_eq!(
            get(0, "c_bit1"),
            Value::Other {
                type_name: "BIT".into(),
                text: "b'1'".into()
            }
        );
        assert!(
            matches!(get(0, "c_bit64"), Value::Other { ref text, .. } if text.starts_with("b'1010") && text.len() == 67)
        );
        assert_eq!(get(0, "c_char"), Value::Text("abc".into()));
        assert_eq!(
            get(0, "c_binary"),
            Value::bytes(hex::decode("00112233445566778899AABBCCDDEEFF").unwrap())
        );
        assert_eq!(
            get(0, "c_varbinary"),
            Value::bytes(vec![0xde, 0xad, 0xbe, 0xef])
        );
        assert_eq!(get(0, "c_blob"), Value::bytes(vec![2, 3]));
        assert_eq!(
            get(0, "c_utf8mb4_bin"),
            Value::Text("Case Sensitive".into())
        );
        assert_eq!(get(0, "c_latin1"), Value::Text("café".into()));
        assert_eq!(get(0, "c_enum"), Value::Text("medium".into()));
        assert_eq!(get(0, "c_set"), Value::Text("a,c".into()));
        assert_eq!(get(0, "c_date"), Value::Date("2024-02-29".into()));
        assert_eq!(
            get(0, "c_datetime"),
            Value::DateTime("2024-02-29 13:37:00.123456".into())
        );
        assert_eq!(get(0, "c_time"), Value::Time("838:59:59.000000".into()));
        assert_eq!(get(2, "c_time"), Value::Time("-838:59:59.000000".into()));
        assert_eq!(get(0, "c_year"), Value::Int(2024));
        assert!(matches!(get(0, "c_geometry"), Value::Bytes { .. }));
        if mariadb {
            assert!(
                matches!(get(0, "c_json"), Value::Text(_)),
                "MariaDB JSON is LONGTEXT"
            );
        } else {
            assert_eq!(
                get(0, "c_json").clone(),
                Value::Json(serde_json::json!({"a": 1, "nested": {"deep": [true, null, 1.5]}}))
            );
        }
        assert!(page.rows[1].iter().skip(1).all(Value::is_null));
        assert!(matches!(
            get(3, "c_longtext"),
            Value::TruncatedText { len: 1_048_576, .. }
        ));
        assert!(
            matches!(get(3, "c_longblob"), Value::Bytes { len: 8192, ref data } if data.len() == 4096)
        );
        let family = |n: &str| page.columns[idx(n)].family;
        assert_eq!(family("c_bool"), TypeFamily::Boolean);
        assert_eq!(family("c_enum"), TypeFamily::Enum);
        assert_eq!(family("c_binary"), TypeFamily::Binary);
        assert_eq!(family("c_utf8mb4_bin"), TypeFamily::Text);
        assert_eq!(family("c_geometry"), TypeFamily::Spatial);

        // Zero dates are shown exactly as stored.
        let zero = d
            .browse_page(&browse(t("tablesail", "zero_dates")), 0, 10)
            .await
            .unwrap();
        assert_eq!(zero.rows[0][1], Value::Date("0000-00-00".into()));
        assert_eq!(
            zero.rows[0][2],
            Value::DateTime("0000-00-00 00:00:00".into())
        );
        assert_eq!(zero.rows[1][1], Value::Date("2024-00-00".into()));
        assert_eq!(zero.rows[2][1], Value::Date("2024-06-15".into()));

        // The full 1 MB value through the export path.
        let mut one = browse(t("tablesail", "all_types"));
        one.filters = vec![Filter {
            column: "id".into(),
            op: FilterOp::Eq,
            value: Some("4".into()),
        }];
        one.columns = vec!["c_longtext".into()];
        let events = drain(d.browse_all(&one).await.unwrap()).await.unwrap();
        let text = events
            .iter()
            .find_map(|e| {
                if let CursorEvent::Rows(r) = e {
                    Some(r[0][0].clone())
                } else {
                    None
                }
            })
            .unwrap();
        assert!(matches!(text, Value::Text(ref s) if s.len() == 1_048_576));

        if mariadb {
            let page = d
                .browse_page(&browse(t("tablesail", "mariadb_types")), 0, 10)
                .await
                .unwrap();
            assert_eq!(page.rows.len(), 2);
            let seq = d.list_tables("tablesail", None).await.unwrap();
            assert!(
                seq.iter()
                    .any(|t| t.table.name == "invoice_seq" && t.kind == TableKind::Sequence)
            );
        }
    }

    async fn suite_streaming(d: Arc<dyn Driver>) {
        let started = Instant::now();
        let mut cursor = d.query(None, "SELECT * FROM big_table").await.unwrap();
        let Some(CursorEvent::Columns(cols)) = cursor.next(100).await.unwrap() else {
            panic!()
        };
        assert_eq!(cols[0].name, "id");
        let Some(CursorEvent::Rows(rows)) = cursor.next(100).await.unwrap() else {
            panic!()
        };
        assert_eq!(rows[0][0], Value::Int(1));
        eprintln!("first row of big_table: {:?}", started.elapsed());
        drop(cursor);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            drain(d.query(None, "SELECT 1 AS one").await.unwrap())
                .await
                .unwrap()
                .len(),
            2
        );

        let mut cursor = d
            .browse_all(&browse(t("tablesail", "big_table")))
            .await
            .unwrap();
        let mut total = 0usize;
        while let Some(e) = cursor.next(10_000).await.unwrap() {
            if let CursorEvent::Rows(r) = e {
                total += r.len();
            }
        }
        assert_eq!(total, 1_000_000);

        // Cancel a long-running statement (KILL QUERY).
        let mut cursor = d
            .query(
                None,
                "SELECT SUM(a.d * b.d + c.d + d.d + e.d + f.d + g.d + h.d + i.d) \
                 FROM digits a, digits b, digits c, digits d, digits e, digits f, digits g, digits h, digits i",
            )
            .await
            .unwrap();
        let cancel = cursor.cancel_handle();
        let started = Instant::now();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            cancel.cancel().await.unwrap();
        });
        let err = loop {
            match cursor.next(10).await {
                Ok(Some(_)) => continue,
                Ok(None) => panic!("finished"),
                Err(e) => break e,
            }
        };
        assert!(matches!(err, Error::Cancelled), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(5));

        let err = drain(d.query(None, "SELECT * FROM no_such_table").await.unwrap())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::Database { message, code: Some(c), .. } if message.contains("doesn't exist") && c == "42S02"),
            "{err:?}"
        );

        // Several result sets from one CALL.
        let events = drain(
            d.query(Some("tablesail"), "CALL multi_results()")
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        let sets = events
            .iter()
            .filter(|e| matches!(e, CursorEvent::Columns(_)))
            .count();
        assert_eq!(sets, 3, "{events:?}");
        assert!(
            matches!(events.last(), Some(CursorEvent::Affected(_))),
            "{events:?}"
        );

        // A database can be chosen per query, and session state sticks.
        let events = drain(
            d.query(Some("tablesail_other"), "SELECT body FROM notes")
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        assert!(
            matches!(&events[1], CursorEvent::Rows(r) if r[0][0] == Value::Text("from another database".into())),
            "{events:?}"
        );
        drain(d.query(None, "SET @tablesail_var = 42").await.unwrap())
            .await
            .unwrap();
        let events = drain(d.query(None, "SELECT @tablesail_var").await.unwrap())
            .await
            .unwrap();
        assert!(
            matches!(&events[1], CursorEvent::Rows(r) if r[0][0] == Value::Int(42)),
            "{events:?}"
        );

        // Empty results still report their columns.
        let empty = drain(
            d.query(None, "SELECT id, email FROM customers WHERE 1 = 0")
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(empty.len(), 1, "{empty:?}");
        assert!(matches!(&empty[0], CursorEvent::Columns(c) if c.len() == 2));
        let write = drain(d.query(None, "DO 1").await.unwrap()).await.unwrap();
        assert_eq!(write, vec![CursorEvent::Affected(0)]);
    }

    async fn suite_write(d: Arc<dyn Driver>) {
        let name = format!("tablesail_scratch_{}", std::process::id());
        let table = t("tablesail", &name);
        let run = |sql: String| {
            let d = d.clone();
            async move { drain(d.query(Some("tablesail"), &sql).await.unwrap()).await }
        };
        run(format!("DROP TABLE IF EXISTS `{name}`")).await.unwrap();
        run(format!(
            "CREATE TABLE `{name}` (id INT AUTO_INCREMENT PRIMARY KEY, label VARCHAR(50), n INT, \
             data JSON NULL, flag TINYINT(1) DEFAULT 1, raw BLOB, d DATE NULL)"
        ))
        .await
        .unwrap();
        let outcome: Result<()> = async {
            let text = |c: &str, v: &str| ColumnInput { column: c.into(), value: CellInput::Text(v.into()) };
            let changes = ChangeSet {
                table: table.clone(),
                changes: vec![
                    RowChange::Insert { values: vec![text("label", "it's \\ here"), text("n", "5"), ColumnInput { column: "flag".into(), value: CellInput::Default }] },
                    RowChange::Insert { values: vec![
                        text("label", "second"),
                        ColumnInput { column: "data".into(), value: CellInput::Json(serde_json::json!({"a": [1, 2]})) },
                        ColumnInput { column: "raw".into(), value: CellInput::Bytes("3q0=".into()) },
                    ] },
                ],
            };
            assert_eq!(d.preview_changes(&changes).await?.len(), 2);
            assert_eq!(d.apply_changes(&changes).await?.rows_affected, 2);
            let page = d.browse_page(&browse(table.clone()), 0, 10).await?;
            assert_eq!(page.rows[0][1], Value::Text("it's \\ here".into()));
            assert_eq!(page.rows[0][4], Value::Bool(true));
            assert_eq!(page.rows[1][5], Value::bytes(vec![0xde, 0xad]));

            let key = |id: i64| vec![ColumnValue { column: "id".into(), value: Value::Int(id) }];
            let update = ChangeSet {
                table: table.clone(),
                changes: vec![RowChange::Update {
                    key: key(1),
                    original: vec![
                        ColumnValue { column: "label".into(), value: Value::Text("it's \\ here".into()) },
                        ColumnValue { column: "data".into(), value: Value::Null },
                    ],
                    set: vec![text("label", "changed"), ColumnInput { column: "n".into(), value: CellInput::Null }],
                }],
            };
            assert_eq!(d.apply_changes(&update).await?.rows_affected, 1);
            let err = d.apply_changes(&update).await.unwrap_err();
            assert!(matches!(&err, Error::Database { message, statement: Some(_), .. } if message == "Row was changed or deleted by someone else"), "{err:?}");
            // Setting a column to the value it already has still counts as a match.
            let same = ChangeSet {
                table: table.clone(),
                changes: vec![RowChange::Update { key: key(1), original: vec![], set: vec![text("label", "changed")] }],
            };
            assert_eq!(d.apply_changes(&same).await?.rows_affected, 1);
            // A JSON original is left out of the optimistic check, the key still applies.
            let json = ChangeSet {
                table: table.clone(),
                changes: vec![RowChange::Update {
                    key: key(2),
                    original: vec![ColumnValue { column: "data".into(), value: Value::Json(serde_json::json!({"a": [1, 2]})) }],
                    set: vec![text("label", "json original")],
                }],
            };
            assert_eq!(d.apply_changes(&json).await?.rows_affected, 1);
            let bad = ChangeSet {
                table: table.clone(),
                changes: vec![
                    RowChange::Delete { key: key(1) },
                    RowChange::Insert { values: vec![text("d", "not a date")] },
                ],
            };
            let err = d.apply_changes(&bad).await.unwrap_err();
            assert!(matches!(&err, Error::Database { statement: Some(_), .. }), "{err:?}");
            assert_eq!(d.count_rows(&browse(table.clone())).await?.exact, Some(2));
            let delete = ChangeSet { table: table.clone(), changes: vec![RowChange::Delete { key: key(1) }] };
            assert_eq!(d.apply_changes(&delete).await?.rows_affected, 1);

            let rows: Vec<Vec<Option<String>>> = (0..1200).map(|i| vec![Some(format!("bulk \\ {i}")), Some(i.to_string())]).collect();
            assert_eq!(d.insert_rows(&table, &["label".into(), "n".into()], rows).await?, 1200);
            assert_eq!(d.count_rows(&browse(table.clone())).await?.exact, Some(1201));
            Ok(())
        }
        .await;
        run(format!("DROP TABLE IF EXISTS `{name}`")).await.unwrap();
        outcome.unwrap();
    }

    macro_rules! live_engine {
        ($($name:ident: $engine:expr, $port:expr, $env:expr, $mariadb:expr;)*) => {$(
            mod $name {
                use super::*;

                #[tokio::test]
                #[ignore = "needs the docker test databases"]
                async fn catalog_and_browsing() {
                    suite_catalog(live($engine, $port, $env).await, $mariadb).await;
                }

                #[tokio::test]
                #[ignore = "needs the docker test databases"]
                async fn value_decoding() {
                    suite_values(live($engine, $port, $env).await, $mariadb).await;
                }

                #[tokio::test]
                #[ignore = "needs the docker test databases"]
                async fn streaming_and_cancel() {
                    suite_streaming(live($engine, $port, $env).await).await;
                }

                #[tokio::test]
                #[ignore = "needs the docker test databases; writes (and drops) a scratch table"]
                async fn write_roundtrip() {
                    suite_write(live($engine, $port, $env).await).await;
                }
            }
        )*};
    }

    live_engine! {
        live_mysql: Engine::Mysql, 23306, "TABLESAIL_MYSQL_URL", false;
        live_mariadb: Engine::Mariadb, 23307, "TABLESAIL_MARIADB_URL", true;
    }
}
