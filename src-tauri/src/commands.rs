//! Every command the UI can invoke. Signatures are the contract with
//! `src/lib/api.ts`; argument names are camelCase on the JS side.
//! Errors serialize as `{ kind, message, code, statement }` (dbcore::Error).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dbcore::driver::CursorEvent;
use dbcore::export::{export, import_csv};
use dbcore::model::{
    ApplyResult, BrowseRequest, ChangeSet, ColumnValue, ConnectionConfig, CsvImport, CsvPreview,
    ExportFormat, ExportSource, Filter, FilterOp, JobProgress, Page, RowCount, SchemaSnapshot,
    Secrets, StatementKind, StatementSpan, TableInfo, TableRef, TableStructure,
};
use dbcore::{Engine, Error, Session, Value};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::jobs::{self, Job};
use crate::runner::{OpenFn, RunningQuery};
use crate::state::AppState;
use crate::{keychain, schema, store};

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

// --- helpers -----------------------------------------------------------------

/// Connecting includes SSH and TLS handshakes; each has its own timeout, this is the backstop.
const CONNECT_BUDGET: Duration = Duration::from_secs(60);

const READ_ONLY: &str = "This connection is read-only.";

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn open_session(config: ConnectionConfig, secrets: Secrets) -> Result<Session> {
    tokio::time::timeout(CONNECT_BUDGET, Session::open(config, secrets))
        .await
        .map_err(|_| Error::Connection("timed out".into()))?
}

fn writable(session: &Session) -> Result<()> {
    if session.config.read_only {
        Err(Error::InvalidInput(READ_ONLY.into()))
    } else {
        Ok(())
    }
}

fn session_info(session: &Session, session_id: &str, databases: Vec<String>) -> SessionInfo {
    SessionInfo {
        session_id: session_id.to_owned(),
        config: session.config.clone(),
        server_version: session.server_version.clone(),
        databases,
    }
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
    store::load(&state.paths()?.config_dir).await
}

/// Saves (creates when `config.id` is empty). `secrets` None keeps stored secrets;
/// a Some field that is an empty string deletes that secret.
#[tauri::command]
pub async fn connection_save(
    state: State<'_, AppState>,
    config: ConnectionConfig,
    secrets: Option<Secrets>,
) -> Result<ConnectionConfig> {
    let mut config = config;
    config.name = config.name.trim().to_owned();
    if config.name.is_empty() {
        return Err(Error::InvalidInput("Give the connection a name.".into()));
    }
    if config.engine == Engine::Sqlite && config.file_path.as_deref().is_none_or(str::is_empty) {
        return Err(Error::InvalidInput("Choose a SQLite database file.".into()));
    }
    if config.id.trim().is_empty() {
        config.id = new_id();
    }

    let dir = &state.paths()?.config_dir;
    let _guard = state.connections_lock.lock().await;
    let mut connections = store::load(dir).await?;
    // Secrets first: if the keychain refuses, nothing half-saved is left behind.
    if let Some(secrets) = &secrets {
        keychain::store(&config.id, secrets).await?;
    }
    match connections.iter_mut().find(|c| c.id == config.id) {
        Some(slot) => *slot = config.clone(),
        None => connections.push(config.clone()),
    }
    store::save(dir, &connections).await?;
    Ok(config)
}

#[tauri::command]
pub async fn connection_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let paths = state.paths()?;
    {
        let _guard = state.connections_lock.lock().await;
        let mut connections = store::load(&paths.config_dir).await?;
        let before = connections.len();
        connections.retain(|c| c.id != id);
        if connections.len() != before {
            store::save(&paths.config_dir, &connections).await?;
        }
    }
    state.close_session(&id).await;
    keychain::delete_all(&id).await;
    schema::remove_cache(&paths.cache_dir, &id).await;
    Ok(())
}

/// Connects (through SSH if configured), reads the version, disconnects.
/// `secrets` None uses the keychain (for an already saved connection).
#[tauri::command]
pub async fn connection_test(
    state: State<'_, AppState>,
    config: ConnectionConfig,
    secrets: Option<Secrets>,
) -> Result<TestResult> {
    let _ = &state;
    let secrets = keychain::merge(keychain::load(&config).await, secrets);
    let started = Instant::now();
    let session = open_session(config, secrets).await?;
    let result = TestResult {
        server_version: session.server_version.clone(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    session.close().await;
    Ok(result)
}

/// Opens (or returns the already open) session for a saved connection.
/// `secrets` overrides the keychain (used when the UI prompted for a password).
#[tauri::command]
pub async fn connection_open(
    state: State<'_, AppState>,
    id: String,
    secrets: Option<Secrets>,
) -> Result<SessionInfo> {
    let open_lock = state.open_lock(&id);
    let _opening = open_lock.lock().await;

    if let Some(session) = state.find_session(&id) {
        match session.driver.list_databases().await {
            Ok(databases) => return Ok(session_info(&session, &id, databases)),
            Err(_) => {
                // The connection died while idle: reconnect instead of returning a corpse.
                drop(session);
                state.close_session(&id).await;
            }
        }
    }

    let config = store::load(&state.paths()?.config_dir)
        .await?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| Error::NotFound("This connection no longer exists.".into()))?;
    let secrets = keychain::merge(keychain::load(&config).await, secrets);
    let session = open_session(config, secrets).await?;
    let databases = match session.driver.list_databases().await {
        Ok(databases) => databases,
        Err(e) => {
            session.close().await;
            return Err(e);
        }
    };
    let session = state.insert_session(id.clone(), session);
    Ok(session_info(&session, &id, databases))
}

#[tauri::command]
pub async fn connection_close(state: State<'_, AppState>, session_id: String) -> Result<()> {
    state.close_session(&session_id).await;
    Ok(())
}

// --- schema ------------------------------------------------------------------

#[tauri::command]
pub async fn schema_databases(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<String>> {
    state.session(&session_id)?.driver.list_databases().await
}

#[tauri::command]
pub async fn schema_schemas(
    state: State<'_, AppState>,
    session_id: String,
    database: String,
) -> Result<Vec<String>> {
    state
        .session(&session_id)?
        .driver
        .list_schemas(&database)
        .await
}

#[tauri::command]
pub async fn schema_tables(
    state: State<'_, AppState>,
    session_id: String,
    database: String,
    schema: Option<String>,
) -> Result<Vec<TableInfo>> {
    state
        .session(&session_id)?
        .driver
        .list_tables(&database, schema.as_deref())
        .await
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
    let session = state.session(&session_id)?;
    schema::snapshot(
        &state.paths()?.cache_dir,
        &session_id,
        &session.driver,
        &database,
        refresh,
    )
    .await
}

#[tauri::command]
pub async fn table_structure(
    state: State<'_, AppState>,
    session_id: String,
    table: TableRef,
) -> Result<TableStructure> {
    state
        .session(&session_id)?
        .driver
        .table_structure(&table)
        .await
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
    state
        .session(&session_id)?
        .driver
        .browse_page(&request, offset, limit)
        .await
}

#[tauri::command]
pub async fn browse_count(
    state: State<'_, AppState>,
    session_id: String,
    request: BrowseRequest,
) -> Result<RowCount> {
    state
        .session(&session_id)?
        .driver
        .count_rows(&request)
        .await
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
    if key.is_empty() {
        return Err(Error::InvalidInput(
            "This row has no primary key, so the cell cannot be located.".into(),
        ));
    }
    let driver = state.session(&session_id)?.driver.clone();
    let request = BrowseRequest {
        table,
        filters: key
            .iter()
            .map(|k| match &k.value {
                Value::Null => Filter {
                    column: k.column.clone(),
                    op: FilterOp::IsNull,
                    value: None,
                },
                value => Filter {
                    column: k.column.clone(),
                    op: FilterOp::Eq,
                    value: Some(value.to_display()),
                },
            })
            .collect(),
        match_all: true,
        raw_where: None,
        sort: Vec::new(),
        columns: vec![column],
    };
    // browse_all (not browse_page) returns complete values; the cursor is dropped
    // after the first row, which stops the statement.
    let mut cursor = driver.browse_all(&request).await?;
    loop {
        match cursor.next(1).await? {
            None => return Err(Error::NotFound("The row no longer exists.".into())),
            Some(CursorEvent::Rows(rows)) => {
                if let Some(mut row) = rows.into_iter().next() {
                    return Ok(if row.is_empty() {
                        Value::Null
                    } else {
                        row.swap_remove(0)
                    });
                }
            }
            Some(_) => {}
        }
    }
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
    let session = state.session(&session_id)?;
    let driver = session.driver.clone();
    let statements = dbcore::split::split_statements(&sql, driver.dialect());
    if statements.is_empty() {
        return Err(Error::InvalidInput("There is no SQL to run.".into()));
    }
    if session.config.read_only && statements.iter().any(|s| s.kind != StatementKind::Read) {
        return Err(Error::InvalidInput(READ_ONLY.into()));
    }

    let open: OpenFn = Arc::new(move |statement: String| {
        let driver = driver.clone();
        let database = database.clone();
        Box::pin(async move { driver.query(database.as_deref(), &statement).await })
    });
    let query_id = new_id();
    state.add_query(RunningQuery::start(
        query_id.clone(),
        session_id,
        open,
        &statements,
    ));
    Ok(QueryRun {
        query_id,
        statements,
    })
}

#[tauri::command]
pub async fn query_status(state: State<'_, AppState>, query_id: String) -> Result<QueryStatus> {
    Ok(state.query(&query_id)?.status())
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
    let query = state.query(&query_id)?;
    query.fetch(statement_index, set_index, offset, limit).await
}

#[tauri::command]
pub async fn query_cancel(state: State<'_, AppState>, query_id: String) -> Result<()> {
    state.query(&query_id)?.cancel().await;
    Ok(())
}

/// Frees a query's buffers and connection.
#[tauri::command]
pub async fn query_close(state: State<'_, AppState>, query_id: String) -> Result<()> {
    if let Some(query) = state.remove_query(&query_id) {
        query.cancel().await;
    }
    Ok(())
}

// --- editing -----------------------------------------------------------------

#[tauri::command]
pub async fn changes_preview(
    state: State<'_, AppState>,
    session_id: String,
    changes: ChangeSet,
) -> Result<Vec<String>> {
    state
        .session(&session_id)?
        .driver
        .preview_changes(&changes)
        .await
}

#[tauri::command]
pub async fn changes_apply(
    state: State<'_, AppState>,
    session_id: String,
    changes: ChangeSet,
) -> Result<ApplyResult> {
    let session = state.session(&session_id)?;
    writable(&session)?;
    session.driver.apply_changes(&changes).await
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
    if path.trim().is_empty() {
        return Err(Error::InvalidInput("Choose a file to export to.".into()));
    }
    let driver = state.session(&session_id)?.driver.clone();
    let job = Job::new();
    let job_id = new_id();
    jobs::spawn(
        job.clone(),
        export(
            driver,
            source,
            format,
            PathBuf::from(path),
            job.progress_fn(),
            job.cancel.clone(),
        ),
    );
    state.add_job(job_id.clone(), job);
    Ok(job_id)
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
    let session = state.session(&session_id)?;
    writable(&session)?;
    let driver = session.driver.clone();
    let job = Job::new();
    let job_id = new_id();
    jobs::spawn(
        job.clone(),
        import_csv(driver, import, job.progress_fn(), job.cancel.clone()),
    );
    state.add_job(job_id.clone(), job);
    Ok(job_id)
}

#[tauri::command]
pub async fn job_status(state: State<'_, AppState>, job_id: String) -> Result<JobProgress> {
    Ok(state.job(&job_id)?.snapshot())
}

#[tauri::command]
pub async fn job_cancel(state: State<'_, AppState>, job_id: String) -> Result<()> {
    state.job(&job_id)?.cancel.cancel();
    Ok(())
}
