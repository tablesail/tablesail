//! PostgreSQL driver. Statements and browse queries run over the simple query
//! protocol (`raw_sql`), so every column arrives as text and is decoded by its type
//! name; no per-type sqlx support is needed and exotic types still display.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{
    PgColumn, PgConnectOptions, PgConnection, PgPool, PgPoolOptions, PgRow, PgSslMode, PgTypeInfo,
    PgTypeKind, Postgres,
};
use sqlx::{
    AssertSqlSafe, Column, Connection, Either, Executor, Row, SqlSafeStr, Statement, TypeInfo,
    ValueRef,
};

use super::sqlcommon::{
    CancelState, ESTIMATE_ONLY_ABOVE, IdleList, Lease, Pump, StructureCache, apply_in_transaction,
    cancel_until_stopped, connection_survives, count_sql, editability, insert_in_transaction,
    insert_statements, is_unfiltered, meta, select_sql, tiebreak_columns,
};
use crate::Engine;
use crate::changes::change_statements;
use crate::cursor::{ChannelCursor, EventSender};
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

const DIALECT: Dialect = Dialect::Postgres;
const DEFAULT_SCHEMA: &str = "public";
/// The database the connection opened in gets room for several editor tabs; the others stay small.
const DEFAULT_DB_CONNECTIONS: u32 = 8;
const OTHER_DB_CONNECTIONS: u32 = 5;

pub async fn connect(params: &ConnectParams) -> Result<Arc<dyn Driver>> {
    let options = pg_options(params)?;
    let default_db = options
        .get_database()
        .map(str::to_owned)
        .unwrap_or_else(|| options.get_username().to_owned());
    let timezone = session_time_zone(params);
    let pool = pool_options(DEFAULT_DB_CONNECTIONS, timezone.clone())
        .connect_with(options.clone())
        .await?;
    let db = Arc::new(Db::new(pool));
    let rows = text_rows(&db.pool, "SELECT current_setting('server_version')").await?;
    let version = cell(&rows[0], 0);
    let version = version.split_whitespace().next().unwrap_or("");
    Ok(Arc::new(PgDriver {
        options,
        dbs: Mutex::new(HashMap::from([(default_db.clone(), db)])),
        timezone,
        default_db,
        version: format!("PostgreSQL {version}"),
        cache: StructureCache::default(),
    }))
}

fn pool_options(max_connections: u32, timezone: Option<String>) -> PgPoolOptions {
    let options = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(15))
        .idle_timeout(Some(Duration::from_secs(600)));
    match timezone {
        // sqlx always sends `TimeZone=UTC` as a startup parameter, which beats a
        // `-c timezone=…` startup option, so the zone is set once connected.
        Some(zone) => options.after_connect(move |conn, _| {
            let set = format!("SET TIME ZONE {}", DIALECT.quote_string(&zone));
            Box::pin(async move {
                sqlx::raw_sql(logged(set)).execute(&mut *conn).await?;
                Ok(())
            })
        }),
        None => options,
    }
}

/// The `timezone` connection option: how timestamptz values are rendered.
fn session_time_zone(params: &ConnectParams) -> Option<String> {
    params
        .config
        .options
        .get("timezone")
        .map(|z| z.trim().to_owned())
        .filter(|z| !z.is_empty())
}

fn pg_options(params: &ConnectParams) -> Result<PgConnectOptions> {
    let cfg = &params.config;
    let mut o = match cfg.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) => url
            .parse::<PgConnectOptions>()
            .map_err(|e| Error::InvalidInput(format!("Invalid connection URL: {e}")))?,
        None => PgConnectOptions::new_without_pgpass(),
    };
    // With a URL the endpoint comes from it, unless an SSH tunnel moved it.
    let endpoint_moved = params.host != cfg.host || params.port != cfg.port;
    if cfg.url.is_none() || endpoint_moved {
        if !params.host.is_empty() {
            o = o.host(&params.host);
        } else if cfg.url.is_none() {
            o = o.host("localhost");
        }
        o = o.port(if params.port != 0 { params.port } else { 5432 });
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
        SslMode::Disable => PgSslMode::Disable,
        SslMode::Prefer => PgSslMode::Prefer,
        SslMode::Require => PgSslMode::Require,
        SslMode::VerifyCa => PgSslMode::VerifyCa,
        SslMode::VerifyFull => PgSslMode::VerifyFull,
    });
    let path = |p: &Option<String>| {
        p.as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
    };
    if let Some(ca) = path(&cfg.ssl.ca_cert) {
        o = o.ssl_root_cert(ca);
    }
    if let Some(cert) = path(&cfg.ssl.client_cert) {
        o = o.ssl_client_cert(cert);
    }
    if let Some(key) = path(&cfg.ssl.client_key) {
        o = o.ssl_client_key(key);
    }

    o = o
        .application_name(
            cfg.options
                .get("application_name")
                .map(String::as_str)
                .unwrap_or("TableSail"),
        )
        // Prepared statements are only used to describe empty results and are
        // deallocated right away; nothing is cached, so nothing can go stale.
        .statement_cache_capacity(0);
    let settings: Vec<(&str, &str)> = ["search_path", "statement_timeout", "lock_timeout", "role"]
        .into_iter()
        .filter_map(|k| cfg.options.get(k).map(|v| (k, v.as_str())))
        .collect();
    if !settings.is_empty() {
        o = o.options(settings);
    }
    Ok(o)
}

/// One database on the server: its own pool (Postgres connections are per database)
/// and its own type cache (type OIDs differ between databases).
struct Db {
    pool: PgPool,
    idle: Arc<IdleList<PoolConnection<Postgres>>>,
    types: Mutex<HashMap<u32, PgLabel>>,
}

impl Db {
    fn new(pool: PgPool) -> Self {
        Db {
            pool,
            idle: Arc::new(IdleList::new(2)),
            types: Mutex::new(HashMap::new()),
        }
    }
}

struct PgDriver {
    options: PgConnectOptions,
    /// Session time zone for every connection (the `timezone` option).
    timezone: Option<String>,
    default_db: String,
    dbs: Mutex<HashMap<String, Arc<Db>>>,
    version: String,
    cache: StructureCache,
}

type PgLease = Lease<PoolConnection<Postgres>>;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PgKind {
    Bool,
    Int,
    Float,
    Decimal,
    Bytea,
    Json,
    Date,
    Time,
    DateTime,
    Uuid,
    Text,
    Other,
}

#[derive(Clone, Debug)]
struct PgLabel {
    name: String,
    kind: PgKind,
    family: TypeFamily,
}

struct PgCol {
    meta: ColumnMeta,
    kind: PgKind,
}

/// Maps a base type name (lowercase, as in `pg_type.typname`/sqlx) to how it is decoded.
fn classify(name: &str) -> (PgKind, TypeFamily) {
    match name {
        "bool" | "boolean" => (PgKind::Bool, TypeFamily::Boolean),
        "int2" | "int4" | "int8" | "oid" => (PgKind::Int, TypeFamily::Integer),
        "float4" | "float8" => (PgKind::Float, TypeFamily::Float),
        "numeric" | "money" => (PgKind::Decimal, TypeFamily::Decimal),
        "bytea" => (PgKind::Bytea, TypeFamily::Binary),
        "json" | "jsonb" => (PgKind::Json, TypeFamily::Json),
        "date" => (PgKind::Date, TypeFamily::Date),
        "time" | "timetz" => (PgKind::Time, TypeFamily::Time),
        "timestamp" | "timestamptz" => (PgKind::DateTime, TypeFamily::DateTime),
        "uuid" => (PgKind::Uuid, TypeFamily::Uuid),
        "text" | "varchar" | "bpchar" | "name" | "char" | "citext" | "xml" => {
            (PgKind::Text, TypeFamily::Text)
        }
        "point" | "line" | "lseg" | "box" | "path" | "polygon" | "circle" | "geometry"
        | "geography" => (PgKind::Other, TypeFamily::Spatial),
        _ => (PgKind::Other, TypeFamily::Other),
    }
}

fn label_of(name: &str) -> PgLabel {
    let (kind, family) = classify(name);
    PgLabel {
        name: name.to_owned(),
        kind,
        family,
    }
}

fn array_label(elem: &str) -> PgLabel {
    PgLabel {
        name: format!("{}[]", elem.trim_start_matches('_')),
        kind: PgKind::Other,
        family: TypeFamily::Array,
    }
}

/// Label from what sqlx knows without a catalog lookup (builtin types, and types
/// resolved by a prepared statement). `None` for an unresolved OID.
fn known_label(ti: &PgTypeInfo) -> Option<PgLabel> {
    let raw = ti.name();
    if raw == "?" {
        return None;
    }
    let name = raw.trim_matches('"').to_ascii_lowercase();
    Some(match ti.kind() {
        PgTypeKind::Simple => label_of(&name),
        PgTypeKind::Enum(_) => PgLabel {
            name,
            kind: PgKind::Text,
            family: TypeFamily::Enum,
        },
        PgTypeKind::Array(_) => array_label(name.trim_end_matches("[]")),
        PgTypeKind::Domain(inner) => return known_label(inner).or_else(|| Some(label_of(&name))),
        PgTypeKind::Composite(_) | PgTypeKind::Range(_) | PgTypeKind::Pseudo => PgLabel {
            name,
            kind: PgKind::Other,
            family: TypeFamily::Other,
        },
    })
}

impl Db {
    /// Resolves the labels of columns, looking up unknown type OIDs (enums, extension
    /// types) in `pg_type` once per database.
    async fn columns_of(&self, columns: &[PgColumn]) -> Result<Vec<PgCol>> {
        let mut labels: Vec<Option<PgLabel>> = Vec::with_capacity(columns.len());
        let mut missing: Vec<u32> = Vec::new();
        {
            let cache = self.types.lock().unwrap_or_else(|p| p.into_inner());
            for c in columns {
                let ti = c.type_info();
                let label = known_label(ti).or_else(|| {
                    let oid = ti.oid()?.0;
                    let hit = cache.get(&oid).cloned();
                    if hit.is_none() && !missing.contains(&oid) {
                        missing.push(oid);
                    }
                    hit
                });
                labels.push(label);
            }
        }
        if !missing.is_empty() {
            // Without catalog access the type still displays, just as "unknown".
            let _ = self.lookup_types(&missing).await;
            let cache = self.types.lock().unwrap_or_else(|p| p.into_inner());
            for (c, label) in columns.iter().zip(labels.iter_mut()) {
                if label.is_none() {
                    *label = c.type_info().oid().and_then(|o| cache.get(&o.0).cloned());
                }
            }
        }
        Ok(columns
            .iter()
            .zip(labels)
            .map(|(c, label)| {
                let label = label.unwrap_or(PgLabel {
                    name: "unknown".to_owned(),
                    kind: PgKind::Other,
                    family: TypeFamily::Other,
                });
                PgCol {
                    meta: meta(c.name(), label.name, label.family),
                    kind: label.kind,
                }
            })
            .collect())
    }

    async fn lookup_types(&self, oids: &[u32]) -> Result<()> {
        let list = oids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        // oid, typname, typtype, typcategory, element name, base name, base typtype
        let rows = text_rows(
            &self.pool,
            &format!(
                "SELECT t.oid::int8::text, t.typname::text, t.typtype::text, t.typcategory::text, \
                        COALESCE(el.typname::text, ''), COALESCE(bt.typname::text, ''), \
                        COALESCE(bt.typtype::text, '') \
                 FROM pg_type t \
                 LEFT JOIN pg_type el ON el.oid = t.typelem \
                 LEFT JOIN pg_type bt ON bt.oid = t.typbasetype AND t.typtype = 'd' \
                 WHERE t.oid::int8 IN ({list})"
            ),
        )
        .await?;
        let mut cache = self.types.lock().unwrap_or_else(|p| p.into_inner());
        for r in rows {
            let Ok(oid) = cell(&r, 0).parse::<u32>() else {
                continue;
            };
            let (typname, typtype, category) = (cell(&r, 1), cell(&r, 2), cell(&r, 3));
            let label = if category == "A" {
                array_label(&cell(&r, 4))
            } else {
                match typtype.as_str() {
                    "e" => PgLabel {
                        name: typname,
                        kind: PgKind::Text,
                        family: TypeFamily::Enum,
                    },
                    "d" if cell(&r, 6) == "e" => PgLabel {
                        name: typname,
                        kind: PgKind::Text,
                        family: TypeFamily::Enum,
                    },
                    "d" => {
                        let mut l = label_of(&cell(&r, 5));
                        l.name = typname;
                        l
                    }
                    "b" => label_of(&typname),
                    // composite, range, multirange, pseudo
                    _ => PgLabel {
                        name: typname,
                        kind: PgKind::Other,
                        family: TypeFamily::Other,
                    },
                }
            };
            cache.insert(oid, label);
        }
        Ok(())
    }
}

fn decode_row(row: &PgRow, cols: &[PgCol], grid: bool) -> Result<DbRow> {
    let mut out = Vec::with_capacity(cols.len());
    for (i, col) in cols.iter().enumerate() {
        let v = decode_cell(row, i, col)?;
        out.push(if grid { v.for_grid() } else { v });
    }
    Ok(out)
}

fn decode_cell(row: &PgRow, i: usize, col: &PgCol) -> Result<Value> {
    let raw = row.try_get_raw(i)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    let s = raw.as_str().map_err(|e| {
        Error::Other(format!(
            "Column \"{}\" is not valid text: {e}",
            col.meta.name
        ))
    })?;
    Ok(decode_text(s, col.kind, &col.meta.type_name))
}

/// Decodes the text form of one value; anything that does not parse stays text.
fn decode_text(s: &str, kind: PgKind, type_name: &str) -> Value {
    match kind {
        PgKind::Bool => Value::Bool(is_true(s)),
        PgKind::Int => s
            .parse::<i64>()
            .map(Value::Int)
            .unwrap_or_else(|_| Value::Text(s.to_owned())),
        PgKind::Float => s
            .parse::<f64>()
            .map(Value::Float)
            .unwrap_or_else(|_| Value::Text(s.to_owned())),
        PgKind::Decimal => Value::Decimal(s.to_owned()),
        PgKind::Bytea => match s.strip_prefix("\\x").map(hex::decode) {
            Some(Ok(data)) => Value::bytes(data),
            _ => Value::Text(s.to_owned()),
        },
        PgKind::Json => serde_json::from_str(s)
            .map(Value::Json)
            .unwrap_or_else(|_| Value::Text(s.to_owned())),
        PgKind::Date => Value::Date(s.to_owned()),
        PgKind::Time => Value::Time(s.to_owned()),
        PgKind::DateTime => Value::DateTime(s.to_owned()),
        PgKind::Uuid => Value::Uuid(s.to_owned()),
        PgKind::Text => Value::Text(s.to_owned()),
        PgKind::Other => Value::Other {
            type_name: type_name.to_owned(),
            text: s.to_owned(),
        },
    }
}

// ---------------------------------------------------------------------------
// Catalog helpers
// ---------------------------------------------------------------------------

type TextRows = Vec<Vec<Option<String>>>;

/// Runs a catalog query over the text protocol; every cell comes back as text.
async fn text_rows(pool: &PgPool, sql: &str) -> Result<TextRows> {
    let rows = sqlx::raw_sql(logged(sql)).fetch_all(pool).await?;
    rows_as_text(&rows)
}

fn rows_as_text(rows: &[PgRow]) -> Result<TextRows> {
    rows.iter()
        .map(|r| {
            (0..r.len())
                .map(|i| {
                    let raw = r.try_get_raw(i)?;
                    if raw.is_null() {
                        return Ok(None);
                    }
                    raw.as_str()
                        .map(|s| Some(s.to_owned()))
                        .map_err(|e| Error::Other(e.to_string()))
                })
                .collect()
        })
        .collect()
}

fn cell(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

fn opt_cell(row: &[Option<String>], i: usize) -> Option<String> {
    row.get(i).cloned().flatten()
}

/// A boolean cast to text is `true`/`false`; the wire format of a bool column is `t`/`f`.
fn is_true(s: &str) -> bool {
    s == "t" || s.eq_ignore_ascii_case("true")
}

fn q(s: &str) -> String {
    DIALECT.quote_string(s)
}

fn qualified(table: &TableRef) -> String {
    DIALECT.qualified(table)
}

/// Fills in the default schema.
fn normalized(table: &TableRef) -> TableRef {
    TableRef {
        database: table.database.clone(),
        schema: Some(
            table
                .schema
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_SCHEMA.to_owned()),
        ),
        name: table.name.clone(),
    }
}

fn kind_of(relkind: &str) -> TableKind {
    match relkind {
        "v" => TableKind::View,
        "m" => TableKind::MaterializedView,
        "f" => TableKind::ForeignTable,
        "S" => TableKind::Sequence,
        _ => TableKind::Table,
    }
}

fn parse_action(code: &str) -> Option<String> {
    Some(
        match code {
            "a" => "NO ACTION",
            "r" => "RESTRICT",
            "c" => "CASCADE",
            "n" => "SET NULL",
            "d" => "SET DEFAULT",
            _ => return None,
        }
        .to_owned(),
    )
}

/// Parses a JSON array of strings produced by `json_agg`/`to_json` in a catalog query.
fn json_strings(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

/// Parses `reltuples::float8::text`; negative means "never analyzed".
fn estimate(s: &str) -> Option<u64> {
    s.parse::<f64>()
        .ok()
        .filter(|n| *n >= 0.0 && n.is_finite())
        .map(|n| n.round() as u64)
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

impl PgDriver {
    fn db(&self, database: &str) -> Arc<Db> {
        let name = if database.is_empty() {
            self.default_db.as_str()
        } else {
            database
        };
        let mut dbs = self.dbs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(db) = dbs.get(name) {
            return db.clone();
        }
        let options = self.options.clone().database(name);
        let db = Arc::new(Db::new(
            pool_options(OTHER_DB_CONNECTIONS, self.timezone.clone()).connect_lazy_with(options),
        ));
        dbs.insert(name.to_owned(), db.clone());
        db
    }

    async fn lease(&self, db: &Arc<Db>) -> Result<PgLease> {
        while let Some(idle) = db.idle.take() {
            let stale = idle.is_stale();
            let mut conn = idle.conn;
            if stale && conn.ping().await.is_err() {
                continue;
            }
            return Ok(Lease::new(conn, idle.id, idle.db, db.idle.clone()));
        }
        let mut conn = db.pool.acquire().await?;
        let row = sqlx::raw_sql("SELECT pg_backend_pid()::text")
            .fetch_one(&mut *conn)
            .await?;
        let pid: u64 = row
            .try_get_raw(0)
            .map_err(Error::from)?
            .as_str()
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::Other("Could not read the backend process id.".to_owned()))?;
        Ok(Lease::new(conn, pid, None, db.idle.clone()))
    }

    async fn start(&self, database: &str, sql: &str, grid: bool) -> Result<Box<dyn Cursor>> {
        let db = self.db(database);
        let mut lease = self.lease(&db).await?;
        let kind = statement_kind(sql, DIALECT);
        if kind == StatementKind::Ddl {
            // The cached table structures may be about to go stale.
            self.cache.clear();
        }
        let state = Arc::new(CancelState::default());
        state.start();
        let cancel = Arc::new(PgCancel {
            pool: db.pool.clone(),
            pid: lease.id,
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
        Ok(Box::new(ChannelCursor::spawn(cancel, move |tx| {
            run_statement(lease, tx, db, sql, grid, kind, state)
        })))
    }

    async fn primary_key(&self, table: &TableRef) -> Result<Arc<Vec<String>>> {
        if let Some(k) = self.cache.primary_key(table) {
            return Ok(k);
        }
        let db = self.db(&table.database);
        let schema = table.schema.as_deref().unwrap_or(DEFAULT_SCHEMA);
        let rows = text_rows(
            &db.pool,
            &format!(
                "SELECT a.attname::text \
                 FROM pg_index i \
                 JOIN pg_class c ON c.oid = i.indrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 CROSS JOIN LATERAL unnest(i.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord) \
                 JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum \
                 WHERE i.indisprimary AND n.nspname = {} AND c.relname = {} \
                 ORDER BY k.ord",
                q(schema),
                q(&table.name)
            ),
        )
        .await?;
        let key = rows.iter().map(|r| cell(r, 0)).collect();
        Ok(self.cache.put_primary_key(table, key))
    }

    async fn structure(&self, table: &TableRef) -> Result<Arc<TableStructure>> {
        let table = normalized(table);
        if let Some(s) = self.cache.structure(&table) {
            return Ok(s);
        }
        let s = self.read_structure(&table).await?;
        Ok(self.cache.put_structure(s))
    }

    async fn read_structure(&self, table: &TableRef) -> Result<TableStructure> {
        let db = self.db(&table.database);
        let pool = &db.pool;
        let schema = table.schema.as_deref().unwrap_or(DEFAULT_SCHEMA);

        let rel = text_rows(
            pool,
            &format!(
                "SELECT c.oid::int8::text, c.relkind::text, \
                        CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = {} AND c.relname = {} AND c.relkind IN ('r','p','v','m','f')",
                q(schema),
                q(&table.name)
            ),
        )
        .await?;
        let Some(rel) = rel.first() else {
            return Err(Error::NotFound(format!("table {schema}.{}", table.name)));
        };
        let oid = cell(rel, 0);
        let relkind = cell(rel, 1);
        let partition_key = opt_cell(rel, 2);
        let kind = kind_of(&relkind);

        // name, formatted type, typname, typtype, typcategory, base typname, base typtype,
        // not null, default, identity, generated, comment, enum type oid
        let col_rows = text_rows(
            pool,
            &format!(
                "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), t.typname::text, \
                        t.typtype::text, t.typcategory::text, COALESCE(bt.typname, t.typname)::text, \
                        COALESCE(bt.typtype, t.typtype)::text, a.attnotnull::text, \
                        pg_get_expr(d.adbin, d.adrelid), a.attidentity::text, a.attgenerated::text, \
                        col_description(a.attrelid, a.attnum), \
                        (CASE WHEN t.typtype = 'd' THEN t.typbasetype ELSE a.atttypid END)::int8::text \
                 FROM pg_attribute a \
                 JOIN pg_type t ON t.oid = a.atttypid \
                 LEFT JOIN pg_type bt ON bt.oid = t.typbasetype AND t.typtype = 'd' \
                 LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                 WHERE a.attrelid = {oid}::oid AND a.attnum > 0 AND NOT a.attisdropped \
                 ORDER BY a.attnum"
            ),
        )
        .await?;

        let enum_oids: Vec<String> = col_rows
            .iter()
            .filter(|r| cell(r, 6) == "e" && cell(r, 4) != "A")
            .map(|r| cell(r, 12))
            .collect();
        let mut enum_values: HashMap<String, Vec<String>> = HashMap::new();
        if !enum_oids.is_empty() {
            let rows = text_rows(
                pool,
                &format!(
                    "SELECT enumtypid::int8::text, enumlabel::text FROM pg_enum \
                     WHERE enumtypid::int8 IN ({}) ORDER BY enumtypid, enumsortorder",
                    enum_oids.join(", ")
                ),
            )
            .await?;
            for r in rows {
                enum_values
                    .entry(cell(&r, 0))
                    .or_default()
                    .push(cell(&r, 1));
            }
        }

        // name, unique, primary, method, column names (JSON)
        let index_rows = text_rows(
            pool,
            &format!(
                "SELECT ic.relname::text, i.indisunique::text, i.indisprimary::text, am.amname::text, \
                        (SELECT COALESCE(json_agg(CASE WHEN k.attnum = 0 \
                                    THEN pg_get_indexdef(i.indexrelid, k.ord::int, true) \
                                    ELSE (SELECT a.attname::text FROM pg_attribute a \
                                          WHERE a.attrelid = i.indrelid AND a.attnum = k.attnum) END \
                                    ORDER BY k.ord), '[]'::json)::text \
                         FROM unnest(i.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord) \
                         WHERE k.ord <= i.indnkeyatts) \
                 FROM pg_index i \
                 JOIN pg_class ic ON ic.oid = i.indexrelid \
                 JOIN pg_am am ON am.oid = ic.relam \
                 WHERE i.indrelid = {oid}::oid \
                 ORDER BY i.indisprimary DESC, ic.relname"
            ),
        )
        .await?;
        let indexes: Vec<IndexInfo> = index_rows
            .iter()
            .map(|r| IndexInfo {
                name: cell(r, 0),
                unique: is_true(&cell(r, 1)),
                primary: is_true(&cell(r, 2)),
                method: Some(cell(r, 3)),
                columns: json_strings(&cell(r, 4)),
            })
            .collect();
        let primary_key = indexes
            .iter()
            .find(|i| i.primary)
            .map(|i| i.columns.clone())
            .unwrap_or_default();

        // name, ref schema, ref table, on update, on delete, columns (JSON), ref columns (JSON)
        let fk_rows = text_rows(
            pool,
            &format!(
                "SELECT con.conname::text, nsp.nspname::text, cl.relname::text, \
                        con.confupdtype::text, con.confdeltype::text, \
                        (SELECT COALESCE(json_agg(a.attname::text ORDER BY k.ord), '[]'::json)::text \
                         FROM unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) \
                         JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum), \
                        (SELECT COALESCE(json_agg(a.attname::text ORDER BY k.ord), '[]'::json)::text \
                         FROM unnest(con.confkey) WITH ORDINALITY AS k(attnum, ord) \
                         JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum) \
                 FROM pg_constraint con \
                 JOIN pg_class cl ON cl.oid = con.confrelid \
                 JOIN pg_namespace nsp ON nsp.oid = cl.relnamespace \
                 WHERE con.conrelid = {oid}::oid AND con.contype = 'f' \
                 ORDER BY con.conname"
            ),
        )
        .await?;
        let foreign_keys: Vec<ForeignKeyInfo> = fk_rows
            .iter()
            .map(|r| ForeignKeyInfo {
                name: cell(r, 0),
                referenced: TableRef {
                    database: table.database.clone(),
                    schema: Some(cell(r, 1)),
                    name: cell(r, 2),
                },
                on_update: parse_action(&cell(r, 3)),
                on_delete: parse_action(&cell(r, 4)),
                columns: json_strings(&cell(r, 5)),
                referenced_columns: json_strings(&cell(r, 6)),
            })
            .collect();

        let columns: Vec<ColumnInfo> = col_rows
            .iter()
            .map(|r| {
                let name = cell(r, 0);
                let default = opt_cell(r, 8);
                let identity = cell(r, 9);
                let is_array = cell(r, 4) == "A";
                let effective_type = cell(r, 6);
                let family = if is_array {
                    TypeFamily::Array
                } else if effective_type == "e" {
                    TypeFamily::Enum
                } else {
                    classify(&cell(r, 5)).1
                };
                ColumnInfo {
                    is_primary_key: primary_key.contains(&name),
                    data_type: cell(r, 1),
                    family,
                    nullable: !is_true(&cell(r, 7)),
                    is_auto_increment: matches!(identity.as_str(), "a" | "d")
                        || default
                            .as_deref()
                            .is_some_and(|d| d.starts_with("nextval(")),
                    is_generated: matches!(cell(r, 10).as_str(), "s" | "v"),
                    enum_values: (family == TypeFamily::Enum)
                        .then(|| enum_values.get(&cell(r, 12)).cloned())
                        .flatten(),
                    default,
                    comment: opt_cell(r, 11).filter(|c| !c.is_empty()),
                    name,
                }
            })
            .collect();

        let ddl = self
            .build_ddl(
                pool,
                &oid,
                table,
                &relkind,
                partition_key.as_deref(),
                &col_rows,
                &index_rows,
            )
            .await
            .ok();

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

    /// Builds a CREATE statement from the catalog (Postgres has no SHOW CREATE TABLE).
    #[allow(clippy::too_many_arguments)]
    async fn build_ddl(
        &self,
        pool: &PgPool,
        oid: &str,
        table: &TableRef,
        relkind: &str,
        partition_key: Option<&str>,
        col_rows: &TextRows,
        _index_rows: &TextRows,
    ) -> Result<String> {
        let name = qualified(table);
        if relkind == "v" || relkind == "m" {
            let def = text_rows(pool, &format!("SELECT pg_get_viewdef({oid}::oid, true)")).await?;
            let keyword = if relkind == "m" {
                "MATERIALIZED VIEW"
            } else {
                "VIEW"
            };
            return Ok(format!("CREATE {keyword} {name} AS\n{}", cell(&def[0], 0)));
        }

        let mut lines: Vec<String> = Vec::new();
        for r in col_rows {
            let mut line = format!("    {} {}", DIALECT.quote_ident(&cell(r, 0)), cell(r, 1));
            let default = opt_cell(r, 8);
            match (cell(r, 9).as_str(), cell(r, 10).as_str(), default) {
                ("a", _, _) => line.push_str(" GENERATED ALWAYS AS IDENTITY"),
                ("d", _, _) => line.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
                (_, "s" | "v", Some(expr)) => {
                    let stored = if cell(r, 10) == "s" {
                        "STORED"
                    } else {
                        "VIRTUAL"
                    };
                    line.push_str(&format!(" GENERATED ALWAYS AS ({expr}) {stored}"));
                }
                (_, _, Some(expr)) => line.push_str(&format!(" DEFAULT {expr}")),
                _ => {}
            }
            if is_true(&cell(r, 7)) {
                line.push_str(" NOT NULL");
            }
            lines.push(line);
        }
        let constraints = text_rows(
            pool,
            &format!(
                "SELECT conname::text, pg_get_constraintdef(oid, true) FROM pg_constraint \
                 WHERE conrelid = {oid}::oid AND contype IN ('p','u','c','f','x') \
                 ORDER BY CASE contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'c' THEN 2 \
                                       WHEN 'x' THEN 3 ELSE 4 END, conname"
            ),
        )
        .await?;
        for c in &constraints {
            lines.push(format!(
                "    CONSTRAINT {} {}",
                DIALECT.quote_ident(&cell(c, 0)),
                cell(c, 1)
            ));
        }
        let head = if relkind == "f" {
            "CREATE FOREIGN TABLE"
        } else {
            "CREATE TABLE"
        };
        let mut ddl = format!("{head} {name} (\n{}\n)", lines.join(",\n"));
        if let Some(key) = partition_key {
            ddl.push_str(&format!("\nPARTITION BY {key}"));
        }
        ddl.push(';');
        let defs = text_rows(
            pool,
            &format!(
                "SELECT pg_get_indexdef(i.indexrelid) FROM pg_index i \
                 WHERE i.indrelid = {oid}::oid AND NOT EXISTS ( \
                     SELECT 1 FROM pg_constraint c \
                     WHERE c.conindid = i.indexrelid AND c.contype IN ('p','u','x')) \
                 ORDER BY 1"
            ),
        )
        .await?;
        for d in &defs {
            ddl.push_str(&format!("\n\n{};", cell(d, 0)));
        }
        Ok(ddl)
    }

    async fn estimate_rows(&self, db: &Db, table: &TableRef) -> Result<Option<u64>> {
        let schema = table.schema.as_deref().unwrap_or(DEFAULT_SCHEMA);
        let rows = text_rows(
            &db.pool,
            &format!(
                "SELECT c.reltuples::float8::text FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = {} AND c.relname = {} AND c.relkind IN ('r','p','m','f')",
                q(schema),
                q(&table.name)
            ),
        )
        .await?;
        Ok(rows.first().and_then(|r| estimate(&cell(r, 0))))
    }
}

/// Cancels the running statement from a second connection.
struct PgCancel {
    pool: PgPool,
    pid: u64,
    state: Arc<CancelState>,
}

#[async_trait]
impl CancelHandle for PgCancel {
    async fn cancel(&self) -> Result<()> {
        if !self.state.request() {
            return Ok(());
        }
        cancel_until_stopped(&self.state, || self.fire()).await
    }
}

impl PgCancel {
    /// One `pg_cancel_backend`.
    async fn fire(&self) -> Result<()> {
        // Only a backend that is running something is cancelled: a repeated cancel must
        // not land on whatever the connection does next.
        sqlx::raw_sql(logged(format!(
            "SELECT pg_cancel_backend(pid) FROM pg_stat_activity \
             WHERE pid = {} AND state = 'active' AND pid <> pg_backend_pid()",
            self.pid
        )))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl Driver for PgDriver {
    fn engine(&self) -> Engine {
        Engine::Postgres
    }

    fn dialect(&self) -> Dialect {
        DIALECT
    }

    async fn server_version(&self) -> Result<String> {
        Ok(self.version.clone())
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        let db = self.db("");
        let rows = text_rows(
            &db.pool,
            "SELECT datname::text FROM pg_database \
             WHERE datallowconn AND NOT datistemplate ORDER BY datname",
        )
        .await?;
        Ok(rows.iter().map(|r| cell(r, 0)).collect())
    }

    async fn list_schemas(&self, database: &str) -> Result<Vec<String>> {
        let db = self.db(database);
        let rows = text_rows(
            &db.pool,
            "SELECT nspname::text FROM pg_namespace \
             WHERE nspname NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
               AND nspname NOT LIKE 'pg\\_temp\\_%' AND nspname NOT LIKE 'pg\\_toast\\_temp\\_%' \
             ORDER BY nspname",
        )
        .await?;
        Ok(rows.iter().map(|r| cell(r, 0)).collect())
    }

    async fn list_tables(&self, database: &str, schema: Option<&str>) -> Result<Vec<TableInfo>> {
        let db = self.db(database);
        let schema = schema.filter(|s| !s.is_empty()).unwrap_or(DEFAULT_SCHEMA);
        let rows = text_rows(
            &db.pool,
            &format!(
                "SELECT c.relname::text, c.relkind::text, c.reltuples::float8::text, \
                        obj_description(c.oid, 'pg_class') \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = {} AND c.relkind IN ('r','p','v','m','f') \
                 ORDER BY c.relname",
                q(schema)
            ),
        )
        .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let relkind = cell(r, 1);
                let kind = kind_of(&relkind);
                TableInfo {
                    table: TableRef {
                        database: database.to_owned(),
                        schema: Some(schema.to_owned()),
                        name: cell(r, 0),
                    },
                    kind,
                    estimated_rows: (relkind != "v").then(|| estimate(&cell(r, 2))).flatten(),
                    comment: opt_cell(r, 3).filter(|c| !c.is_empty()),
                }
            })
            .collect())
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        let table = normalized(table);
        let s = self.read_structure(&table).await?;
        Ok((*self.cache.put_structure(s)).clone())
    }

    async fn browse_page(&self, request: &BrowseRequest, offset: u64, limit: u32) -> Result<Page> {
        let started = Instant::now();
        let mut request = request.clone();
        request.table = normalized(&request.table);
        let db = self.db(&request.table.database);
        // The key orders unsorted pages and breaks ties between sorted ones; a table
        // without one is tie-broken by its columns.
        let mut key = self.primary_key(&request.table).await?;
        if key.is_empty() && !request.sort.is_empty() {
            key = Arc::new(tiebreak_columns(&*self.structure(&request.table).await?));
        }
        let sql = select_sql(DIALECT, &request, &key, Some((limit, offset)))?;
        let rows = sqlx::raw_sql(logged(sql.clone()))
            .fetch_all(&db.pool)
            .await?;
        let cols = match rows.first() {
            Some(first) => db.columns_of(first.columns()).await?,
            None => {
                // No row to read types from: use the table's catalog columns.
                let structure = self.structure(&request.table).await?;
                structure
                    .columns
                    .iter()
                    .filter(|c| request.columns.is_empty() || request.columns.contains(&c.name))
                    .map(|c| PgCol {
                        meta: meta(&c.name, &c.data_type, c.family),
                        kind: PgKind::Other,
                    })
                    .collect()
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
        request.table = normalized(&request.table);
        let db = self.db(&request.table.database);
        let estimate = if is_unfiltered(&request) {
            self.estimate_rows(&db, &request.table)
                .await
                .unwrap_or(None)
        } else {
            None
        };
        if estimate.is_some_and(|n| n > ESTIMATE_ONLY_ABOVE) {
            return Ok(RowCount {
                exact: None,
                estimate,
            });
        }
        let sql = count_sql(DIALECT, &request)?;
        let rows = text_rows(&db.pool, &sql).await?;
        let exact = cell(&rows[0], 0).parse::<u64>().ok();
        Ok(RowCount { exact, estimate })
    }

    async fn browse_all(&self, request: &BrowseRequest) -> Result<Box<dyn Cursor>> {
        let mut request = request.clone();
        request.table = normalized(&request.table);
        let sql = select_sql(DIALECT, &request, &[], None)?;
        self.start(&request.table.database, &sql, false).await
    }

    async fn query(&self, database: Option<&str>, sql: &str) -> Result<Box<dyn Cursor>> {
        self.start(database.unwrap_or(""), sql, true).await
    }

    async fn preview_changes(&self, changes: &ChangeSet) -> Result<Vec<String>> {
        let mut changes = changes.clone();
        changes.table = normalized(&changes.table);
        let structure = self.structure(&changes.table).await?;
        change_statements(DIALECT, &structure, &changes)
    }

    async fn apply_changes(&self, changes: &ChangeSet) -> Result<ApplyResult> {
        let mut changes = changes.clone();
        changes.table = normalized(&changes.table);
        let structure = self.structure(&changes.table).await?;
        let statements = change_statements(DIALECT, &structure, &changes)?;
        let db = self.db(&changes.table.database);
        apply_in_transaction(&db.pool, &statements, &changes.changes).await
    }

    async fn insert_rows(
        &self,
        table: &TableRef,
        columns: &[String],
        rows: Vec<Vec<Option<String>>>,
    ) -> Result<u64> {
        let table = normalized(table);
        let statements = insert_statements(DIALECT, &table, columns, &rows)?;
        let db = self.db(&table.database);
        insert_in_transaction(&db.pool, &statements).await
    }

    async fn close(&self) {
        self.cache.clear();
        let dbs: Vec<Arc<Db>> = {
            let mut map = self.dbs.lock().unwrap_or_else(|p| p.into_inner());
            map.drain().map(|(_, db)| db).collect()
        };
        for db in dbs {
            db.idle.clear();
            db.pool.close().await;
        }
    }
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

async fn run_statement(
    mut lease: PgLease,
    tx: EventSender,
    db: Arc<Db>,
    sql: String,
    grid: bool,
    kind: StatementKind,
    state: Arc<CancelState>,
) -> Result<()> {
    let mut pump = Pump::new(tx, grid);
    let outcome = stream_statement(&mut lease, &db, &sql, kind, &mut pump).await;
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
    lease: &mut PgLease,
    db: &Db,
    sql: &str,
    kind: StatementKind,
    pump: &mut Pump,
) -> Result<()> {
    let conn: &mut PgConnection = lease.conn();
    let mut cols: Vec<PgCol> = Vec::new();
    let mut deferred: Option<u64> = None;
    {
        let mut stream = sqlx::raw_sql(logged(sql)).fetch_many(&mut *conn);
        while let Some(item) = stream.next().await {
            match item? {
                Either::Right(row) => {
                    if !pump.in_set() {
                        cols = db.columns_of(row.columns()).await?;
                        pump.columns(cols.iter().map(|c| c.meta.clone()).collect())
                            .await?;
                    }
                    pump.row(decode_row(&row, &cols, pump.grid())?).await?;
                }
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
    if let Some(affected) = deferred {
        // A query that returned no rows still has columns; ask the server to describe it.
        match (&mut *conn)
            .prepare(AssertSqlSafe(sql.to_owned()).into_sql_str())
            .await
        {
            Ok(stmt) if !stmt.columns().is_empty() => {
                let cols = db.columns_of(stmt.columns()).await?;
                pump.columns(cols.into_iter().map(|c| c.meta).collect())
                    .await?;
                pump.end_set(0, false).await?;
            }
            Ok(_) => pump.affected(affected).await?,
            Err(_) => pump.affected(affected).await?,
        }
        // Named statements are never cached (capacity 0), so drop the one just made.
        let _ = sqlx::raw_sql("DEALLOCATE ALL").execute(&mut *conn).await;
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
            name: "pg".into(),
            engine: Engine::Postgres,
            host: "db.example.com".into(),
            port: 5433,
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
        let o = pg_options(&params(config())).unwrap();
        assert_eq!(o.get_host(), "db.example.com");
        assert_eq!(o.get_port(), 5433);
        assert_eq!(o.get_username(), "alice");
        assert_eq!(o.get_database(), Some("shop"));
        assert_eq!(o.get_application_name(), Some("TableSail"));
    }

    #[test]
    fn tunnel_endpoint_wins() {
        let mut p = params(config());
        p.host = "127.0.0.1".into();
        p.port = 40000;
        let o = pg_options(&p).unwrap();
        assert_eq!(o.get_host(), "127.0.0.1");
        assert_eq!(o.get_port(), 40000);
    }

    #[test]
    fn url_is_the_base() {
        let mut c = config();
        c.host = String::new();
        c.port = 0;
        c.user = String::new();
        c.database = String::new();
        c.url = Some("postgres://bob@pg.internal:6543/orders?sslmode=require".into());
        let o = pg_options(&params(c.clone())).unwrap();
        assert_eq!(o.get_host(), "pg.internal");
        assert_eq!(o.get_port(), 6543);
        assert_eq!(o.get_username(), "bob");
        assert_eq!(o.get_database(), Some("orders"));
        // A tunnel overrides the endpoint only.
        let mut p = params(c);
        p.host = "127.0.0.1".into();
        p.port = 41000;
        let o = pg_options(&p).unwrap();
        assert_eq!(o.get_host(), "127.0.0.1");
        assert_eq!(o.get_port(), 41000);
        assert_eq!(o.get_database(), Some("orders"));
        let mut bad = config();
        bad.url = Some("not a url".into());
        assert!(matches!(
            pg_options(&params(bad)),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn ssl_modes_map() {
        for (mode, expected) in [
            (SslMode::Disable, PgSslMode::Disable),
            (SslMode::Prefer, PgSslMode::Prefer),
            (SslMode::Require, PgSslMode::Require),
            (SslMode::VerifyCa, PgSslMode::VerifyCa),
            (SslMode::VerifyFull, PgSslMode::VerifyFull),
        ] {
            let mut c = config();
            c.ssl.mode = mode;
            assert!(
                matches!(pg_options(&params(c)).unwrap().get_ssl_mode(), m if std::mem::discriminant(&m) == std::mem::discriminant(&expected))
            );
        }
    }

    #[test]
    fn classifies_types() {
        assert_eq!(classify("int8"), (PgKind::Int, TypeFamily::Integer));
        assert_eq!(classify("numeric"), (PgKind::Decimal, TypeFamily::Decimal));
        assert_eq!(classify("money"), (PgKind::Decimal, TypeFamily::Decimal));
        assert_eq!(
            classify("timestamptz"),
            (PgKind::DateTime, TypeFamily::DateTime)
        );
        assert_eq!(classify("citext"), (PgKind::Text, TypeFamily::Text));
        assert_eq!(classify("point"), (PgKind::Other, TypeFamily::Spatial));
        assert_eq!(classify("interval"), (PgKind::Other, TypeFamily::Other));
        assert_eq!(classify("hstore"), (PgKind::Other, TypeFamily::Other));
        assert_eq!(array_label("_int4").name, "int4[]");
        assert_eq!(array_label("mood").family, TypeFamily::Array);
    }

    #[test]
    fn decodes_text_values() {
        use PgKind::*;
        assert_eq!(decode_text("t", Bool, "bool"), Value::Bool(true));
        assert_eq!(decode_text("f", Bool, "bool"), Value::Bool(false));
        assert_eq!(
            decode_text("9007199254740993", Int, "int8"),
            Value::Int(9_007_199_254_740_993)
        );
        assert_eq!(decode_text("-1.5", Float, "float8"), Value::Float(-1.5));
        assert!(matches!(decode_text("NaN", Float, "float8"), Value::Float(f) if f.is_nan()));
        assert_eq!(
            decode_text("Infinity", Float, "float4"),
            Value::Float(f64::INFINITY)
        );
        assert_eq!(
            decode_text("-Infinity", Float, "float4"),
            Value::Float(f64::NEG_INFINITY)
        );
        assert_eq!(
            decode_text("12345.6789012345", Decimal, "numeric"),
            Value::Decimal("12345.6789012345".into())
        );
        assert_eq!(
            decode_text("NaN", Decimal, "numeric"),
            Value::Decimal("NaN".into())
        );
        assert_eq!(
            decode_text("\\xdeadbeef", Bytea, "bytea"),
            Value::bytes(vec![0xde, 0xad, 0xbe, 0xef])
        );
        assert_eq!(decode_text("\\x", Bytea, "bytea"), Value::bytes(vec![]));
        assert_eq!(
            decode_text("{\"a\": [1, 2]}", Json, "jsonb"),
            Value::Json(serde_json::json!({"a": [1, 2]}))
        );
        assert_eq!(
            decode_text("not json", Json, "json"),
            Value::Text("not json".into())
        );
        assert_eq!(
            decode_text("infinity", DateTime, "timestamptz"),
            Value::DateTime("infinity".into())
        );
        assert_eq!(
            decode_text("5874897-12-31", Date, "date"),
            Value::Date("5874897-12-31".into())
        );
        assert_eq!(
            decode_text("{1,2}", Other, "int4[]"),
            Value::Other {
                type_name: "int4[]".into(),
                text: "{1,2}".into()
            }
        );
    }

    #[test]
    fn parses_catalog_values() {
        assert_eq!(estimate("1e+06"), Some(1_000_000));
        assert_eq!(estimate("-1"), None);
        assert_eq!(estimate("12345"), Some(12345));
        assert_eq!(json_strings("[\"a\",\"b c\"]"), ["a", "b c"]);
        assert!(json_strings("garbage").is_empty());
        assert_eq!(parse_action("c").as_deref(), Some("CASCADE"));
        assert_eq!(parse_action("x"), None);
        assert_eq!(kind_of("m"), TableKind::MaterializedView);
        let t = normalized(&TableRef {
            database: "d".into(),
            schema: None,
            name: "t".into(),
        });
        assert_eq!(t.schema.as_deref(), Some("public"));
    }

    // --- Live tests against the docker "nasty schema" (cargo test -- --ignored) -------------

    use crate::driver::CursorEvent;
    use crate::model::{ColumnInput, ColumnValue, Editability, Filter, FilterOp, RowChange, Sort};
    use crate::value::CellInput;

    async fn live() -> Arc<dyn Driver> {
        let mut c = config();
        c.host = "127.0.0.1".into();
        c.port = 25432;
        c.user = "tablesail".into();
        c.database = "tablesail".into();
        if let Ok(url) = std::env::var("TABLESAIL_POSTGRES_URL") {
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

    fn t(schema: &str, name: &str) -> TableRef {
        TableRef {
            database: "tablesail".into(),
            schema: Some(schema.into()),
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

    #[tokio::test]
    #[ignore = "needs the docker test databases"]
    async fn live_catalog_and_browsing() {
        let d = live().await;
        assert!(
            d.server_version()
                .await
                .unwrap()
                .starts_with("PostgreSQL 1")
        );
        assert!(
            d.list_databases()
                .await
                .unwrap()
                .contains(&"tablesail".to_owned())
        );
        let schemas = d.list_schemas("tablesail").await.unwrap();
        for s in ["public", "billing", "Mixed Case Schema", "empty_schema"] {
            assert!(schemas.contains(&s.to_owned()), "{schemas:?}");
        }
        assert!(!schemas.contains(&"pg_catalog".to_owned()));
        assert!(
            d.list_tables("tablesail", Some("empty_schema"))
                .await
                .unwrap()
                .is_empty()
        );

        let tables = d.list_tables("tablesail", Some("public")).await.unwrap();
        let find = |n: &str| {
            tables
                .iter()
                .find(|t| t.table.name == n)
                .unwrap_or_else(|| panic!("missing {n}"))
        };
        assert_eq!(find("customer_totals").kind, TableKind::View);
        assert_eq!(find("mood_counts").kind, TableKind::MaterializedView);
        assert_eq!(find("big_table").kind, TableKind::Table);
        assert!(find("big_table").estimated_rows.unwrap() > 900_000);
        assert!(
            tables
                .iter()
                .any(|t| t.table.name == "weird \"quoted\" name; DROP TABLE x;--")
        );

        // Every table's structure and first page must work.
        for info in &tables {
            let name = &info.table.name;
            let s = d
                .table_structure(&t("public", name))
                .await
                .unwrap_or_else(|e| panic!("structure of {name}: {e}"));
            assert!(!s.columns.is_empty(), "{name}");
            assert!(s.ddl.is_some(), "ddl of {name}");
            let page = d
                .browse_page(&browse(t("public", name)), 0, 25)
                .await
                .unwrap_or_else(|e| panic!("browse {name}: {e}"));
            assert_eq!(page.columns.len(), s.columns.len(), "{name}");
            d.count_rows(&browse(t("public", name)))
                .await
                .unwrap_or_else(|e| panic!("count {name}: {e}"));
        }

        let orders = d.table_structure(&t("public", "orders")).await.unwrap();
        assert_eq!(orders.primary_key, ["id"]);
        assert_eq!(orders.editability, Editability::Editable);
        let col = |n: &str| orders.columns.iter().find(|c| c.name == n).unwrap();
        assert!(col("id").is_auto_increment);
        assert_eq!(col("status").family, TypeFamily::Enum);
        assert_eq!(
            col("status").enum_values.as_deref(),
            Some(
                &[
                    "sad".to_owned(),
                    "ok".into(),
                    "happy".into(),
                    "ünïcødé 🎉".into()
                ][..]
            )
        );
        assert_eq!(col("total").data_type, "numeric(12,2)");
        assert_eq!(col("placed_at").family, TypeFamily::DateTime);
        assert!(!col("total").nullable);
        assert_eq!(orders.foreign_keys.len(), 1);
        assert_eq!(orders.foreign_keys[0].referenced.name, "customers");
        assert_eq!(orders.foreign_keys[0].on_delete.as_deref(), Some("CASCADE"));
        assert!(
            orders
                .indexes
                .iter()
                .any(|i| i.name == "orders_customer_idx" && i.columns == ["customer_id"])
        );
        assert!(
            orders
                .ddl
                .unwrap()
                .contains("CREATE TABLE \"public\".\"orders\"")
        );

        let computed = d.table_structure(&t("public", "computed")).await.unwrap();
        assert!(
            computed
                .columns
                .iter()
                .find(|c| c.name == "total")
                .unwrap()
                .is_generated
        );
        assert!(computed.columns[0].is_auto_increment);

        let no_pk = d.table_structure(&t("public", "no_pk")).await.unwrap();
        assert!(matches!(no_pk.editability, Editability::ReadOnly(_)));
        let view = d
            .table_structure(&t("public", "customer_totals"))
            .await
            .unwrap();
        assert_eq!(
            view.editability,
            Editability::ReadOnly("Views can't be edited.".into())
        );
        let composite = d
            .table_structure(&t("public", "composite_pk"))
            .await
            .unwrap();
        assert_eq!(composite.primary_key, ["tenant_id", "item_id"]);
        let other_schema = d.table_structure(&t("billing", "invoices")).await.unwrap();
        assert_eq!(other_schema.columns[1].data_type, "numeric(12,2)");

        // Counts: exact for small tables, estimate-only above a million rows.
        let small = d
            .count_rows(&browse(t("public", "customers")))
            .await
            .unwrap();
        assert_eq!(small.exact, Some(2));
        let big = d
            .count_rows(&browse(t("public", "big_table")))
            .await
            .unwrap();
        assert!(
            big.exact.is_none() || big.exact == Some(1_000_000),
            "{big:?}"
        );
        let mut filtered = browse(t("public", "big_table"));
        filtered.filters = vec![Filter {
            column: "name".into(),
            op: FilterOp::StartsWith,
            value: Some("row 99999".into()),
        }];
        assert_eq!(d.count_rows(&filtered).await.unwrap().exact, Some(11));

        // Paging a 1M-row table by key is fast and stable.
        let started = Instant::now();
        let page = d
            .browse_page(&browse(t("public", "big_table")), 500_000, 100)
            .await
            .unwrap();
        assert_eq!(page.rows[0][0], Value::Int(500_001));
        eprintln!("big_table page at offset 500000: {:?}", started.elapsed());
        let mut sorted = browse(t("public", "big_table"));
        sorted.sort = vec![Sort {
            column: "id".into(),
            descending: true,
        }];
        let page = d.browse_page(&sorted, 0, 3).await.unwrap();
        assert_eq!(page.rows[0][0], Value::Int(1_000_000));
    }

    #[tokio::test]
    #[ignore = "needs the docker test databases"]
    async fn live_value_decoding() {
        let d = live().await;
        let mut r = browse(t("public", "all_types"));
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
        let row1 = &page.rows[0];
        let get = |row: &Vec<Value>, n: &str| row[idx(n)].clone();
        assert_eq!(get(row1, "c_bigint"), Value::Int(9_007_199_254_740_993));
        assert_eq!(get(row1, "c_boolean"), Value::Bool(true));
        assert_eq!(get(row1, "c_real"), Value::Float(1.5));
        assert!(
            matches!(get(row1, "c_numeric"), Value::Decimal(ref s) if s.starts_with("12345.6789"))
        );
        assert!(matches!(get(row1, "c_money"), Value::Decimal(_)));
        assert_eq!(
            get(row1, "c_bytea"),
            Value::bytes(vec![0xde, 0xad, 0xbe, 0xef])
        );
        assert!(matches!(get(row1, "c_jsonb"), Value::Json(_)));
        assert!(matches!(get(row1, "c_json"), Value::Json(_)));
        assert_eq!(
            get(row1, "c_uuid"),
            Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into())
        );
        assert_eq!(get(row1, "c_date"), Value::Date("2024-02-29".into()));
        assert!(matches!(get(row1, "c_timestamptz"), Value::DateTime(_)));
        assert!(matches!(get(row1, "c_time"), Value::Time(_)));
        assert_eq!(get(row1, "c_mood"), Value::Text("happy".into()));
        assert!(
            matches!(get(row1, "c_int_array"), Value::Other { ref type_name, ref text } if type_name == "int4[]" && text == "{1,2,3}")
        );
        assert!(matches!(get(row1, "c_mood_array"), Value::Other { .. }));
        assert!(matches!(get(row1, "c_hstore"), Value::Other { .. }));
        assert!(matches!(get(row1, "c_interval"), Value::Other { .. }));
        assert!(matches!(get(row1, "c_address"), Value::Other { .. }));
        assert_eq!(get(row1, "c_citext"), Value::Text("CaseInsensitive".into()));
        assert_eq!(get(row1, "c_domain"), Value::Int(7));
        let family = |n: &str| page.columns[idx(n)].family;
        assert_eq!(family("c_mood"), TypeFamily::Enum);
        assert_eq!(family("c_int_array"), TypeFamily::Array);
        assert_eq!(family("c_point"), TypeFamily::Spatial);
        assert_eq!(family("c_citext"), TypeFamily::Text);
        assert_eq!(family("c_uuid"), TypeFamily::Uuid);

        // Row 2 is all NULL; row 3 holds NaN, infinities and empty values.
        assert!(page.rows[1].iter().skip(1).all(Value::is_null));
        let row3 = &page.rows[2];
        assert!(matches!(get(row3, "c_numeric_free"), Value::Decimal(ref s) if s == "NaN"));
        assert_eq!(get(row3, "c_real"), Value::Float(f64::NEG_INFINITY));
        assert_eq!(get(row3, "c_double"), Value::Float(f64::INFINITY));
        assert_eq!(
            get(row3, "c_timestamp"),
            Value::DateTime("-infinity".into())
        );
        assert_eq!(get(row3, "c_bytea"), Value::bytes(vec![]));
        assert_eq!(get(row3, "c_bigint"), Value::Int(i64::MIN));
        // Row 4: the 1 MB text is cut for the grid; the full value comes from browse_all.
        let row4 = &page.rows[3];
        assert!(matches!(
            get(row4, "c_text"),
            Value::TruncatedText { len: 1_048_576, .. }
        ));
        assert!(
            matches!(get(row4, "c_bytea"), Value::Bytes { len: 8192, ref data } if data.len() == 4096)
        );
        let mut one = browse(t("public", "all_types"));
        one.filters = vec![Filter {
            column: "id".into(),
            op: FilterOp::Eq,
            value: Some("4".into()),
        }];
        one.columns = vec!["c_text".into()];
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

        // Types with a user-defined OID resolve through pg_type even on an empty result.
        let events = drain(
            d.query(
                Some("tablesail"),
                "SELECT c_mood, c_hstore FROM all_types WHERE false",
            )
            .await
            .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        let CursorEvent::Columns(cols) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(cols[0].type_name, "mood");
        assert_eq!(cols[0].family, TypeFamily::Enum);
        assert_eq!(cols[1].type_name, "hstore");
    }

    #[tokio::test]
    #[ignore = "needs the docker test databases"]
    async fn live_streaming_and_cancel() {
        let d = live().await;
        // First row of a 1M-row table arrives long before the table is read.
        let started = Instant::now();
        let mut cursor = d
            .query(None, "SELECT * FROM big_table ORDER BY id")
            .await
            .unwrap();
        let Some(CursorEvent::Columns(cols)) = cursor.next(100).await.unwrap() else {
            panic!()
        };
        assert_eq!(cols[0].name, "id");
        let Some(CursorEvent::Rows(rows)) = cursor.next(100).await.unwrap() else {
            panic!()
        };
        assert_eq!(rows[0][0], Value::Int(1));
        eprintln!("first row of big_table: {:?}", started.elapsed());
        // Dropping the cursor mid-stream cancels the server side and frees the connection.
        drop(cursor);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let events = drain(d.query(None, "SELECT 1 AS one").await.unwrap())
            .await
            .unwrap();
        assert_eq!(events.len(), 2, "{events:?}");

        // Full export-style read.
        let mut cursor = d
            .browse_all(&browse(t("public", "big_table")))
            .await
            .unwrap();
        let mut total = 0usize;
        while let Some(e) = cursor.next(10_000).await.unwrap() {
            if let CursorEvent::Rows(r) = e {
                total += r.len();
            }
        }
        assert_eq!(total, 1_000_000);

        // Cancel a long-running statement.
        let mut cursor = d.query(None, "SELECT pg_sleep(30)").await.unwrap();
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

        // Server errors carry the message; transactions span statements; kinds of events.
        let err = drain(d.query(None, "SELECT * FROM no_such_table").await.unwrap())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::Database { message, code: Some(c), .. } if message.contains("does not exist") && c == "42P01"),
            "{err:?}"
        );
        let events = drain(
            d.query(None, "SET application_name = 'tablesail-test'")
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(events, vec![CursorEvent::Affected(0)]);
        let events = drain(d.query(None, "SHOW application_name").await.unwrap())
            .await
            .unwrap();
        let CursorEvent::Rows(rows) = &events[1] else {
            panic!("{events:?}")
        };
        assert_eq!(
            rows[0][0],
            Value::Text("tablesail-test".into()),
            "session settings persist"
        );
        let empty = drain(
            d.query(None, "SELECT id, name FROM customers WHERE false")
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(empty.len(), 1);
        assert!(matches!(&empty[0], CursorEvent::Columns(c) if c.len() == 2));
    }

    #[tokio::test]
    #[ignore = "needs the docker test databases; writes (and drops) a scratch table"]
    async fn live_write_roundtrip() {
        let d = live().await;
        let name = format!("tablesail_scratch_{}", std::process::id());
        let table = t("public", &name);
        let run = |sql: String| {
            let d = d.clone();
            async move { drain(d.query(None, &sql).await.unwrap()).await }
        };
        run(format!("DROP TABLE IF EXISTS \"{name}\""))
            .await
            .unwrap();
        run(format!(
            "CREATE TABLE \"{name}\" (id serial PRIMARY KEY, label text, n integer, data jsonb, flag boolean DEFAULT true, raw bytea)"
        ))
        .await
        .unwrap();
        let outcome: Result<()> = async {
            let text = |c: &str, v: &str| ColumnInput { column: c.into(), value: CellInput::Text(v.into()) };
            let changes = ChangeSet {
                table: table.clone(),
                changes: vec![
                    RowChange::Insert { values: vec![text("label", "it's"), text("n", "5"), ColumnInput { column: "flag".into(), value: CellInput::Default }] },
                    RowChange::Insert { values: vec![
                        text("label", "second"),
                        ColumnInput { column: "data".into(), value: CellInput::Json(serde_json::json!({"a": [1, 2]})) },
                        ColumnInput { column: "raw".into(), value: CellInput::Bytes("3q0=".into()) },
                    ] },
                ],
            };
            let preview = d.preview_changes(&changes).await?;
            assert_eq!(preview.len(), 2);
            assert_eq!(d.apply_changes(&changes).await?.rows_affected, 2);
            let page = d.browse_page(&browse(table.clone()), 0, 10).await?;
            assert_eq!(page.rows.len(), 2);
            assert_eq!(page.rows[0][1], Value::Text("it's".into()));
            assert_eq!(page.rows[0][4], Value::Bool(true));
            assert_eq!(page.rows[1][5], Value::bytes(vec![0xde, 0xad]));

            let key = |id: i64| vec![ColumnValue { column: "id".into(), value: Value::Int(id) }];
            let update = ChangeSet {
                table: table.clone(),
                changes: vec![RowChange::Update {
                    key: key(1),
                    original: vec![
                        ColumnValue { column: "label".into(), value: Value::Text("it's".into()) },
                        ColumnValue { column: "data".into(), value: Value::Null },
                    ],
                    set: vec![text("label", "changed"), ColumnInput { column: "n".into(), value: CellInput::Null }],
                }],
            };
            assert_eq!(d.apply_changes(&update).await?.rows_affected, 1);
            // The same update again no longer matches: refused, nothing changes.
            let err = d.apply_changes(&update).await.unwrap_err();
            assert!(matches!(&err, Error::Database { message, statement: Some(_), .. } if message == "Row was changed or deleted by someone else"), "{err:?}");
            let jsonb = ChangeSet {
                table: table.clone(),
                changes: vec![RowChange::Update {
                    key: key(2),
                    original: vec![ColumnValue { column: "data".into(), value: Value::Json(serde_json::json!({"a": [1, 2]})) }],
                    set: vec![text("label", "jsonb original matched")],
                }],
            };
            assert_eq!(d.apply_changes(&jsonb).await?.rows_affected, 1);
            // One failing statement rolls back the earlier ones.
            let bad = ChangeSet {
                table: table.clone(),
                changes: vec![
                    RowChange::Delete { key: key(1) },
                    RowChange::Insert { values: vec![text("n", "not a number")] },
                ],
            };
            let err = d.apply_changes(&bad).await.unwrap_err();
            assert!(matches!(&err, Error::Database { statement: Some(_), .. }), "{err:?}");
            assert_eq!(d.count_rows(&browse(table.clone())).await?.exact, Some(2));
            let delete = ChangeSet { table: table.clone(), changes: vec![RowChange::Delete { key: key(1) }] };
            assert_eq!(d.apply_changes(&delete).await?.rows_affected, 1);

            let rows: Vec<Vec<Option<String>>> = (0..1200).map(|i| vec![Some(format!("bulk {i}")), Some(i.to_string())]).collect();
            assert_eq!(d.insert_rows(&table, &["label".into(), "n".into()], rows).await?, 1200);
            assert_eq!(d.count_rows(&browse(table.clone())).await?.exact, Some(1201));
            Ok(())
        }
        .await;
        run(format!("DROP TABLE IF EXISTS \"{name}\""))
            .await
            .unwrap();
        outcome.unwrap();
    }
}
