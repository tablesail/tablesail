//! Every command the UI can invoke. Signatures are the contract with
//! `src/lib/api.ts`; argument names are camelCase on the JS side.
//! Errors serialize as `{ kind, message, code, statement }` (dbcore::Error).

use dbcore::model::{
    ApplyResult, BrowseRequest, ChangeSet, ColumnValue, ConnectionConfig, CsvImport, CsvPreview,
    ExportFormat, ExportSource, JobProgress, Page, RowCount, SchemaSnapshot, Secrets,
    StatementSpan, TableInfo, TableRef, TableStructure,
};
use dbcore::{Engine, Error, Value};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

type Result<T> = std::result::Result<T, Error>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreInfo {
    pub version: &'static str,
    pub engines: Vec<Engine>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub server_version: String,
    pub elapsed_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// Equal to the connection id (one session per saved connection).
    pub session_id: String,
    pub config: ConnectionConfig,
    pub server_version: String,
    pub databases: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRun {
    pub query_id: String,
    pub statements: Vec<StatementSpan>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum StatementState {
    Pending,
    Running,
    Done,
    Error,
    Cancelled,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResultSetStatus {
    pub set_index: usize,
    pub columns: Vec<dbcore::model::ColumnMeta>,
    /// Rows buffered so far.
    pub loaded: u64,
    /// The server has sent every row (or reading stopped, see `truncated`).
    pub complete: bool,
    /// Reading stopped early: a non-final statement of a script is read up to
    /// 10,000 rows and then closed so it doesn't hold a connection.
    pub truncated: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatementStatus {
    pub index: usize,
    pub sql: String,
    pub state: StatementState,
    pub error: Option<Error>,
    pub affected: Option<u64>,
    pub sets: Vec<ResultSetStatus>,
    pub elapsed_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryStatus {
    pub query_id: String,
    pub finished: bool,
    pub statements: Vec<StatementStatus>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowsPage {
    pub rows: Vec<dbcore::Row>,
    pub offset: u64,
    pub loaded: u64,
    pub complete: bool,
}

// --- app -------------------------------------------------------------------

#[tauri::command]
pub fn app_info() -> CoreInfo {
    CoreInfo {
        version: dbcore::VERSION,
        engines: Engine::ALL.to_vec(),
    }
}

// --- saved connections (config JSON in the app config dir, secrets in the OS keychain)

#[tauri::command]
pub async fn connections_list(state: State<'_, AppState>) -> Result<Vec<ConnectionConfig>> {
    todo!("A3 {:p}", &state)
}

/// Saves (creates when `config.id` is empty). `secrets` None keeps stored secrets;
/// a Some field that is an empty string deletes that secret.
#[tauri::command]
pub async fn connection_save(
    state: State<'_, AppState>,
    config: ConnectionConfig,
    secrets: Option<Secrets>,
) -> Result<ConnectionConfig> {
    todo!("A3 {:p} {config:?} {secrets:?}", &state)
}

#[tauri::command]
pub async fn connection_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    todo!("A3 {:p} {id}", &state)
}

/// Connects (through SSH if configured), reads the version, disconnects.
/// `secrets` None uses the keychain (for an already saved connection).
#[tauri::command]
pub async fn connection_test(
    state: State<'_, AppState>,
    config: ConnectionConfig,
    secrets: Option<Secrets>,
) -> Result<TestResult> {
    todo!("A3 {:p} {config:?} {secrets:?}", &state)
}

/// Opens (or returns the already open) session for a saved connection.
/// `secrets` overrides the keychain (used when the UI prompted for a password).
#[tauri::command]
pub async fn connection_open(
    state: State<'_, AppState>,
    id: String,
    secrets: Option<Secrets>,
) -> Result<SessionInfo> {
    todo!("A3 {:p} {id} {secrets:?}", &state)
}

#[tauri::command]
pub async fn connection_close(state: State<'_, AppState>, session_id: String) -> Result<()> {
    todo!("A3 {:p} {session_id}", &state)
}

// --- schema ------------------------------------------------------------------

#[tauri::command]
pub async fn schema_databases(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<String>> {
    todo!("A3 {:p} {session_id}", &state)
}

#[tauri::command]
pub async fn schema_schemas(
    state: State<'_, AppState>,
    session_id: String,
    database: String,
) -> Result<Vec<String>> {
    todo!("A3 {:p} {session_id} {database}", &state)
}

#[tauri::command]
pub async fn schema_tables(
    state: State<'_, AppState>,
    session_id: String,
    database: String,
    schema: Option<String>,
) -> Result<Vec<TableInfo>> {
    todo!("A3 {:p} {session_id} {database} {schema:?}", &state)
}

/// Tables and column names of a whole database. `refresh` false returns the
/// on-disk cache when present (instant second launch); true re-reads the server
/// and rewrites the cache.
#[tauri::command]
pub async fn schema_snapshot(
    state: State<'_, AppState>,
    session_id: String,
    database: String,
    refresh: bool,
) -> Result<SchemaSnapshot> {
    todo!("A3 {:p} {session_id} {database} {refresh}", &state)
}

#[tauri::command]
pub async fn table_structure(
    state: State<'_, AppState>,
    session_id: String,
    table: TableRef,
) -> Result<TableStructure> {
    todo!("A3 {:p} {session_id} {table:?}", &state)
}

// --- browsing ----------------------------------------------------------------

#[tauri::command]
pub async fn browse_page(
    state: State<'_, AppState>,
    session_id: String,
    request: BrowseRequest,
    offset: u64,
    limit: u32,
) -> Result<Page> {
    todo!("A3 {:p} {session_id} {request:?} {offset} {limit}", &state)
}

#[tauri::command]
pub async fn browse_count(
    state: State<'_, AppState>,
    session_id: String,
    request: BrowseRequest,
) -> Result<RowCount> {
    todo!("A3 {:p} {session_id} {request:?}", &state)
}

/// Full (untruncated) value of one cell, located by primary key.
#[tauri::command]
pub async fn cell_fetch(
    state: State<'_, AppState>,
    session_id: String,
    table: TableRef,
    key: Vec<ColumnValue>,
    column: String,
) -> Result<Value> {
    todo!("A3 {:p} {session_id} {table:?} {key:?} {column}", &state)
}

// --- SQL editor --------------------------------------------------------------

#[tauri::command]
pub fn sql_split(sql: String, engine: Engine) -> Vec<StatementSpan> {
    dbcore::split::split_statements(&sql, dbcore::Dialect::for_engine(engine))
}

/// Splits `sql` and runs the statements one after another in the background.
/// Execution stops at the first error. Rejected with InvalidInput on a
/// read-only connection when any statement is not a Read.
#[tauri::command]
pub async fn query_run(
    state: State<'_, AppState>,
    session_id: String,
    database: Option<String>,
    sql: String,
) -> Result<QueryRun> {
    todo!("A3 {:p} {session_id} {database:?} {sql}", &state)
}

#[tauri::command]
pub async fn query_status(state: State<'_, AppState>, query_id: String) -> Result<QueryStatus> {
    todo!("A3 {:p} {query_id}", &state)
}

/// Rows `[offset, offset + limit)` of one result set. Waits (up to ~5 s) until
/// those rows are loaded or the set is complete, pulling from the cursor on
/// demand — rows are only read from the server as the user scrolls.
#[tauri::command]
pub async fn query_fetch(
    state: State<'_, AppState>,
    query_id: String,
    statement_index: usize,
    set_index: usize,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    todo!(
        "A3 {:p} {query_id} {statement_index} {set_index} {offset} {limit}",
        &state
    )
}

#[tauri::command]
pub async fn query_cancel(state: State<'_, AppState>, query_id: String) -> Result<()> {
    todo!("A3 {:p} {query_id}", &state)
}

/// Frees a query's buffers and connection.
#[tauri::command]
pub async fn query_close(state: State<'_, AppState>, query_id: String) -> Result<()> {
    todo!("A3 {:p} {query_id}", &state)
}

// --- editing -----------------------------------------------------------------

#[tauri::command]
pub async fn changes_preview(
    state: State<'_, AppState>,
    session_id: String,
    changes: ChangeSet,
) -> Result<Vec<String>> {
    todo!("A3 {:p} {session_id} {changes:?}", &state)
}

#[tauri::command]
pub async fn changes_apply(
    state: State<'_, AppState>,
    session_id: String,
    changes: ChangeSet,
) -> Result<ApplyResult> {
    todo!("A3 {:p} {session_id} {changes:?}", &state)
}

// --- export / import (background jobs) ---------------------------------------

#[tauri::command]
pub async fn export_start(
    state: State<'_, AppState>,
    session_id: String,
    source: ExportSource,
    format: ExportFormat,
    path: String,
) -> Result<String> {
    todo!("A3 {:p} {session_id} {source:?} {format:?} {path}", &state)
}

#[tauri::command]
pub async fn csv_preview(path: String, delimiter: Option<char>) -> Result<CsvPreview> {
    dbcore::export::csv_preview(&path, delimiter, 20)
}

#[tauri::command]
pub async fn import_start(
    state: State<'_, AppState>,
    session_id: String,
    import: CsvImport,
) -> Result<String> {
    todo!("A3 {:p} {session_id} {import:?}", &state)
}

#[tauri::command]
pub async fn job_status(state: State<'_, AppState>, job_id: String) -> Result<JobProgress> {
    todo!("A3 {:p} {job_id}", &state)
}

#[tauri::command]
pub async fn job_cancel(state: State<'_, AppState>, job_id: String) -> Result<()> {
    todo!("A3 {:p} {job_id}", &state)
}
