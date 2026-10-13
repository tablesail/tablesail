//! Data model shared by the drivers, the Tauri commands and the UI.
//! Every type here is mirrored in `src/lib/types.ts`; keep them in sync.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Engine;
use crate::value::{CellInput, Row, Value};

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

/// A saved connection. Secrets are never stored here; they live in the OS keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    /// Stable id (UUID); empty on a not-yet-saved connection.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub engine: Engine,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    /// Default database; empty means the server default (or "all" in the sidebar).
    #[serde(default)]
    pub database: String,
    /// SQLite database file.
    #[serde(default)]
    pub file_path: Option<String>,
    /// Full connection URL (e.g. `mongodb+srv://…`); when set it overrides host/port/user/database.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub ssl: SslConfig,
    #[serde(default)]
    pub ssh: Option<SshConfig>,
    #[serde(default)]
    pub env: EnvTag,
    /// Optional custom accent colour (`#rrggbb`); defaults come from `env`.
    #[serde(default)]
    pub color: Option<String>,
    /// Block every write from the UI (edits, imports, non-SELECT statements).
    #[serde(default)]
    pub read_only: bool,
    /// Engine-specific extras (e.g. `authSource` for Mongo, `timezone` for MySQL).
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvTag {
    #[default]
    Local,
    Development,
    Staging,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SslConfig {
    #[serde(default)]
    pub mode: SslMode,
    /// Paths to PEM files.
    #[serde(default)]
    pub ca_cert: Option<String>,
    #[serde(default)]
    pub client_cert: Option<String>,
    #[serde(default)]
    pub client_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfig {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    /// Optional bastion this hop is reached through.
    #[serde(default)]
    pub jump: Option<Box<SshConfig>>,
}

fn default_ssh_port() -> u16 {
    22
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SshAuth {
    /// Password from the keychain.
    Password,
    /// Private key file; passphrase (if any) from the keychain.
    Key { path: String },
    /// ssh-agent via `SSH_AUTH_SOCK`.
    Agent,
}

/// Secrets supplied with a save/test, or loaded from the keychain on open.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Secrets {
    #[serde(default)]
    pub db_password: Option<String>,
    #[serde(default)]
    pub ssh_password: Option<String>,
    #[serde(default)]
    pub ssh_passphrase: Option<String>,
    /// Secrets for the jump host, if any.
    #[serde(default)]
    pub jump_password: Option<String>,
    #[serde(default)]
    pub jump_passphrase: Option<String>,
}

/// What a driver's `connect` receives: the config plus the endpoint actually dialled
/// (127.0.0.1:<local port> when an SSH tunnel is in front).
#[derive(Debug, Clone)]
pub struct ConnectParams {
    pub config: ConnectionConfig,
    pub password: Option<String>,
    pub host: String,
    pub port: u16,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// Identifies a table, view or collection. `schema` is None for engines without
/// schemas (MySQL, SQLite's attached databases use `database`, Mongo).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRef {
    pub database: String,
    #[serde(default)]
    pub schema: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TableKind {
    Table,
    View,
    MaterializedView,
    ForeignTable,
    Virtual,
    Sequence,
    Collection,
    TimeSeries,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    #[serde(flatten)]
    pub table: TableRef,
    pub kind: TableKind,
    /// Row-count estimate from statistics (cheap), when the engine has one.
    pub estimated_rows: Option<u64>,
    pub comment: Option<String>,
}

/// Normalized column type family, so the UI can pick editors and alignment
/// without knowing every engine's type names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeFamily {
    Boolean,
    Integer,
    Float,
    Decimal,
    Text,
    Binary,
    Json,
    Date,
    Time,
    DateTime,
    Uuid,
    Enum,
    Array,
    Spatial,
    ObjectId,
    Document,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    /// Type as the engine names it, with modifiers (`varchar(255)`, `numeric(12,2)`, `tinyint(1)`).
    pub data_type: String,
    pub family: TypeFamily,
    pub nullable: bool,
    /// Default expression as text.
    pub default: Option<String>,
    pub is_primary_key: bool,
    pub is_auto_increment: bool,
    pub is_generated: bool,
    /// Allowed values for enum/set columns.
    pub enum_values: Option<Vec<String>>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    /// e.g. btree, gin, hash, fulltext.
    pub method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced: TableRef,
    pub referenced_columns: Vec<String>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason", rename_all = "camelCase")]
pub enum Editability {
    Editable,
    /// Not editable, with a sentence the UI shows as-is ("Views can't be edited.").
    ReadOnly(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableStructure {
    pub table: TableRef,
    pub kind: TableKind,
    pub columns: Vec<ColumnInfo>,
    /// Primary key columns in key order (Mongo: `["_id"]`).
    pub primary_key: Vec<String>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub editability: Editability,
    /// CREATE statement (or collection options as JSON for Mongo), when available.
    pub ddl: Option<String>,
}

/// Everything the sidebar and autocomplete need for one database, cached on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSnapshot {
    pub database: String,
    /// Schema name → tables. Engines without schemas use a single "" key.
    pub schemas: BTreeMap<String, Vec<TableInfo>>,
    /// Table name (schema-qualified when the engine has schemas) → column names, for autocomplete.
    pub columns: BTreeMap<String, Vec<String>>,
    /// Unix milliseconds.
    pub fetched_at: u64,
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// A result column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    /// Engine type name (`int8`, `VARCHAR`, `BOOLEAN`, Mongo: the most common BSON type).
    pub type_name: String,
    pub family: TypeFamily,
}

/// A page of rows from a table browse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Row>,
    pub offset: u64,
    /// SQL that produced the page (shown in the UI's status bar); Mongo: the find as JSON.
    pub query: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowCount {
    /// Exact count, when it was cheap enough to compute.
    pub exact: Option<u64>,
    /// Statistics estimate, when available.
    pub estimate: Option<u64>,
}

// ---------------------------------------------------------------------------
// Browsing: filters and sorting pushed down to the server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterOp {
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    /// Substring, case-insensitive where the engine supports it.
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    /// Raw LIKE pattern.
    Like,
    /// Comma-separated list.
    In,
    NotIn,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub column: String,
    pub op: FilterOp,
    /// Ignored for IsNull/IsNotNull.
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sort {
    pub column: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseRequest {
    pub table: TableRef,
    #[serde(default)]
    pub filters: Vec<Filter>,
    /// true: all filters must match (AND); false: any (OR).
    #[serde(default = "yes")]
    pub match_all: bool,
    /// Raw WHERE clause (SQL) or filter document (Mongo, relaxed JSON); ANDed with `filters`.
    #[serde(default)]
    pub raw_where: Option<String>,
    #[serde(default)]
    pub sort: Vec<Sort>,
    /// Restrict to these columns; empty means all.
    #[serde(default)]
    pub columns: Vec<String>,
}

fn yes() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnValue {
    pub column: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInput {
    pub column: String,
    pub value: CellInput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RowChange {
    Insert {
        values: Vec<ColumnInput>,
    },
    Update {
        /// Primary key values as loaded.
        key: Vec<ColumnValue>,
        /// Original values of the changed columns, for the optimistic WHERE clause.
        original: Vec<ColumnValue>,
        set: Vec<ColumnInput>,
    },
    Delete {
        key: Vec<ColumnValue>,
    },
}

/// Pending edits to one table, committed in one transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    pub table: TableRef,
    pub changes: Vec<RowChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub statements: u64,
    pub rows_affected: u64,
}

// ---------------------------------------------------------------------------
// Script splitting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatementKind {
    /// SELECT, SHOW, EXPLAIN, WITH … SELECT, find/aggregate.
    Read,
    /// INSERT, UPDATE, DELETE, MERGE, insertOne…
    Write,
    /// CREATE, ALTER, DROP, TRUNCATE, GRANT…
    Ddl,
    /// BEGIN, COMMIT, SET, USE…
    Other,
}

/// One statement of a script, with byte offsets into the original text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementSpan {
    /// Byte offsets (UTF-8) of the statement, excluding the terminator and surrounding whitespace.
    pub start: usize,
    pub end: usize,
    pub sql: String,
    pub kind: StatementKind,
    /// DROP, TRUNCATE, ALTER … DROP, DELETE/UPDATE without WHERE, deleteMany({}), drop().
    pub destructive: bool,
}

// ---------------------------------------------------------------------------
// Export / import
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportSource {
    Table {
        request: BrowseRequest,
    },
    Query {
        database: Option<String>,
        sql: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportFormat {
    Csv {
        #[serde(default = "comma")]
        delimiter: char,
        #[serde(default = "yes")]
        header: bool,
        /// Text written for NULL.
        #[serde(default)]
        null_as: String,
    },
    /// JSON array of objects.
    Json {
        #[serde(default)]
        pretty: bool,
    },
    /// Newline-delimited JSON (one object per line).
    JsonLines,
    /// INSERT statements.
    Sql {
        /// Table name used in the INSERTs.
        table: String,
        #[serde(default = "default_batch")]
        rows_per_insert: u32,
    },
}

fn comma() -> char {
    ','
}

fn default_batch() -> u32 {
    100
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: char,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImport {
    pub path: String,
    pub table: TableRef,
    #[serde(default = "comma")]
    pub delimiter: char,
    #[serde(default = "yes")]
    pub has_header: bool,
    /// For each CSV column, the target column (None = skip).
    pub mapping: Vec<Option<String>>,
    /// CSV text treated as NULL (e.g. "" or "NULL").
    #[serde(default)]
    pub null_as: Option<String>,
}

/// Progress of a long-running background job (export, import).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub rows: u64,
    pub bytes: u64,
    pub done: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}
