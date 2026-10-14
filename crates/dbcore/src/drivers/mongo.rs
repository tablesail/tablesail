//! MongoDB driver on the official `mongodb` crate.
//!
//! Collections are presented as tables: columns are inferred by sampling, rows
//! are documents, `_id` is the primary key. Queries are mongosh statements.

mod changes;
mod convert;
mod exec;
mod filter;
mod infer;
mod render;
mod shell;
mod structure;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt, stream};
use mongodb::bson::{Bson, Document, doc};
use mongodb::options::{ClientOptions, Credential, Tls, TlsOptions};
use mongodb::{Client, Collection, Database};
use tokio_util::sync::CancellationToken;

use crate::Engine;
use crate::cursor::ChannelCursor;
use crate::dialect::Dialect;
use crate::driver::{Cursor, Driver};
use crate::error::{Error, Result};
use crate::model::{
    ApplyResult, BrowseRequest, ChangeSet, ColumnInfo, ColumnMeta, ConnectParams, Page, RowCount,
    SslConfig, SslMode, TableInfo, TableKind, TableRef, TableStructure,
};
use crate::querylog::record;
use crate::value::Row;

use convert::{bson_to_value, db_err, error_code, missing};

/// Fails fast when the server is unreachable or the credentials are wrong.
const SERVER_SELECTION_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest an exact filtered count may run before we fall back to the estimate.
const COUNT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an inferred structure is reused (paging asks for it on every page).
const STRUCTURE_TTL: Duration = Duration::from_secs(60);
/// Server error code of an operation that exceeded `maxTimeMS`.
const MAX_TIME_EXPIRED: i32 = 50;

type CacheEntry = (Instant, Arc<TableStructure>);

/// Inferred structures per collection, valid for `STRUCTURE_TTL`.
#[derive(Default)]
struct StructureCache {
    map: Mutex<HashMap<(String, String), CacheEntry>>,
}

impl StructureCache {
    fn get(&self, table: &TableRef) -> Option<Arc<TableStructure>> {
        let map = self.map.lock().ok()?;
        let (at, structure) = map.get(&(table.database.clone(), table.name.clone()))?;
        (at.elapsed() < STRUCTURE_TTL).then(|| structure.clone())
    }

    fn put(&self, table: &TableRef, structure: Arc<TableStructure>) {
        if let Ok(mut map) = self.map.lock() {
            map.insert(
                (table.database.clone(), table.name.clone()),
                (Instant::now(), structure),
            );
        }
    }

    fn invalidate(&self, database: &str, name: &str) {
        if let Ok(mut map) = self.map.lock() {
            map.remove(&(database.to_owned(), name.to_owned()));
        }
    }

    fn clear(&self) {
        if let Ok(mut map) = self.map.lock() {
            map.clear();
        }
    }
}

pub struct MongoDriver {
    client: Client,
    /// Database named in the connection, if any.
    configured_db: Option<String>,
    /// Database used when a table or statement names none.
    default_db: String,
    /// Replica sets and sharded clusters; standalone servers cannot run transactions.
    supports_transactions: bool,
    structures: Arc<StructureCache>,
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

/// Percent-encodes everything outside the URI "unreserved" set.
fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn build_uri(params: &ConnectParams) -> String {
    let cfg = &params.config;
    let mut uri = String::from("mongodb://");
    if !cfg.user.is_empty() {
        uri.push_str(&encode_component(&cfg.user));
        if let Some(password) = params.password.as_deref().filter(|p| !p.is_empty()) {
            uri.push(':');
            uri.push_str(&encode_component(password));
        }
        uri.push('@');
    }
    let host = if params.host.is_empty() {
        "localhost"
    } else {
        params.host.as_str()
    };
    if host.contains(':') && !host.starts_with('[') {
        uri.push_str(&format!("[{host}]"));
    } else {
        uri.push_str(host);
    }
    let port = if params.port == 0 {
        Engine::Mongodb.default_port()
    } else {
        params.port
    };
    uri.push_str(&format!(":{port}/"));

    let mut pairs: Vec<String> = Vec::new();
    let mut has_topology = false;
    for (key, value) in &cfg.options {
        if key.trim().is_empty() || value.trim().is_empty() {
            continue;
        }
        let lower = key.to_ascii_lowercase();
        has_topology |= lower == "directconnection" || lower == "replicaset";
        pairs.push(format!(
            "{}={}",
            encode_component(key.trim()),
            encode_component(value.trim())
        ));
    }
    // One endpoint (often an SSH tunnel): talk to exactly that server instead of
    // chasing the member addresses a replica set advertises.
    if !has_topology {
        pairs.push("directConnection=true".into());
    }
    uri.push('?');
    uri.push_str(&pairs.join("&"));
    uri
}

/// A private temp file holding client certificate and key together (the driver
/// wants one PEM). Removed on drop.
struct TempPem(PathBuf);

impl TempPem {
    fn create(cert: &str, key: &str) -> Result<TempPem> {
        use std::io::Write;
        let mut pem = std::fs::read_to_string(cert)
            .map_err(|e| Error::Io(format!("client certificate {cert}: {e}")))?;
        if !pem.ends_with('\n') {
            pem.push('\n');
        }
        pem.push_str(
            &std::fs::read_to_string(key)
                .map_err(|e| Error::Io(format!("client key {key}: {e}")))?,
        );
        let path = std::env::temp_dir().join(format!("tablesail-{}.pem", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(pem.as_bytes())?;
        Ok(TempPem(path))
    }
}

impl Drop for TempPem {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn non_empty(text: &Option<String>) -> Option<&str> {
    text.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

/// Applies the SSL settings. `Prefer` leaves whatever the URI (or `mongodb+srv`) says.
///
/// `Require` encrypts without verifying the server unless a CA file is given,
/// like libpq's `require`. The rustls backend cannot skip only the hostname
/// check, so `VerifyCa` verifies the hostname as well.
fn apply_tls(options: &mut ClientOptions, ssl: &SslConfig) -> Result<Option<TempPem>> {
    match ssl.mode {
        SslMode::Prefer => return Ok(None),
        SslMode::Disable => {
            options.tls = Some(Tls::Disabled);
            return Ok(None);
        }
        SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {}
    }
    let mut tls = TlsOptions::default();
    let ca = non_empty(&ssl.ca_cert);
    tls.ca_file_path = ca.map(PathBuf::from);
    if ssl.mode == SslMode::Require && ca.is_none() {
        tls.allow_invalid_certificates = Some(true);
    }
    let mut temp = None;
    match (non_empty(&ssl.client_cert), non_empty(&ssl.client_key)) {
        (Some(cert), Some(key)) if cert != key => {
            let pem = TempPem::create(cert, key)?;
            tls.cert_key_file_path = Some(pem.0.clone());
            temp = Some(pem);
        }
        (Some(file), _) | (None, Some(file)) => tls.cert_key_file_path = Some(PathBuf::from(file)),
        (None, None) => {}
    }
    options.tls = Some(Tls::Enabled(tls));
    Ok(temp)
}

pub async fn connect(params: &ConnectParams) -> Result<Arc<dyn Driver>> {
    let cfg = &params.config;
    let uri = match non_empty(&cfg.url) {
        Some(url) => url.to_owned(),
        None => build_uri(params),
    };
    let mut options = ClientOptions::parse(&uri).await.map_err(db_err)?;
    options
        .server_selection_timeout
        .get_or_insert(SERVER_SELECTION_TIMEOUT);
    options
        .connect_timeout
        .get_or_insert(SERVER_SELECTION_TIMEOUT);
    options.app_name.get_or_insert_with(|| "TableSail".into());

    // Credentials that did not travel in the URI (the password lives in the keychain).
    let password = params.password.clone().filter(|p| !p.is_empty());
    match options.credential.as_mut() {
        Some(credential) => {
            if credential.password.is_none() {
                credential.password = password;
            }
        }
        None if !cfg.user.is_empty() => {
            let mut credential = Credential::default();
            credential.username = Some(cfg.user.clone());
            credential.password = password;
            credential.source = cfg
                .options
                .get("authSource")
                .filter(|s| !s.trim().is_empty())
                .cloned();
            options.credential = Some(credential);
        }
        None => {}
    }

    let temp_pem = apply_tls(&mut options, &cfg.ssl)?;
    let client = Client::with_options(options).map_err(db_err)?;
    // The driver reads certificate files while building the client.
    drop(temp_pem);

    // `isMaster` is understood by every server version; it also forces the
    // connection and authentication, so failures surface here.
    let hello = client
        .database("admin")
        .run_command(doc! { "isMaster": 1 })
        .await
        .map_err(db_err)?;
    let sharded = hello.get_str("msg").is_ok_and(|m| m == "isdbgrid");
    let supports_transactions = hello.contains_key("setName") || sharded;

    let configured_db = Some(cfg.database.trim().to_owned()).filter(|d| !d.is_empty());
    Ok(Arc::new(MongoDriver {
        client,
        default_db: configured_db.clone().unwrap_or_else(|| "test".into()),
        configured_db,
        supports_transactions,
        structures: Arc::new(StructureCache::default()),
    }))
}

// ---------------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------------

/// Cancels a running statement: stops the producer task and asks the server to
/// kill the operations tagged with this statement's comment.
pub(super) struct MongoCancel {
    client: Client,
    comment: String,
    token: CancellationToken,
}

impl MongoCancel {
    pub(super) fn new(client: Client, comment: String, token: CancellationToken) -> Arc<Self> {
        Arc::new(MongoCancel {
            client,
            comment,
            token,
        })
    }
}

#[async_trait]
impl crate::driver::CancelHandle for MongoCancel {
    async fn cancel(&self) -> Result<()> {
        // Dropping the producer's future closes its connection; killOp then
        // stops the server-side work. The kill is best effort (it needs privileges).
        self.token.cancel();
        let kill = async {
            let admin = self.client.database("admin");
            let reply = admin
                .run_command(doc! {
                    "currentOp": 1,
                    "$ownOps": true,
                    "$or": [
                        { "command.comment": &self.comment },
                        { "cursor.originatingCommand.comment": &self.comment },
                    ],
                })
                .await?;
            for op in reply.get_array("inprog").into_iter().flatten() {
                if let Some(id) = op.as_document().and_then(|d| d.get("opid")) {
                    let _ = admin
                        .run_command(doc! { "killOp": 1, "op": id.clone() })
                        .await;
                }
            }
            Ok::<(), mongodb::error::Error>(())
        };
        let _ = tokio::time::timeout(Duration::from_secs(5), kill).await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

impl MongoDriver {
    fn database(&self, name: &str) -> Database {
        if name.is_empty() {
            self.client.database(&self.default_db)
        } else {
            self.client.database(name)
        }
    }

    fn collection(&self, table: &TableRef) -> Collection<Document> {
        self.database(&table.database).collection(&table.name)
    }

    /// Documents of a `listCollections`-style cursor command.
    async fn cursor_command(&self, database: &str, command: Document) -> Result<Vec<Document>> {
        self.database(database)
            .run_cursor_command(command)
            .await
            .map_err(db_err)?
            .try_collect()
            .await
            .map_err(db_err)
    }
}

/// Rows of `docs` laid out in `columns`; fields a document lacks are `missing`.
fn rows_of(docs: &[Document], columns: &[ColumnMeta], grid: bool) -> Vec<Row> {
    docs.iter()
        .map(|doc| {
            columns
                .iter()
                .map(|c| {
                    let value = doc.get(&c.name).map_or_else(missing, bson_to_value);
                    if grid { value.for_grid() } else { value }
                })
                .collect()
        })
        .collect()
}

/// `base` columns followed by any keys of `docs` that `base` lacks.
fn merge_columns(base: &[ColumnMeta], docs: &[Document]) -> Vec<ColumnMeta> {
    let mut columns = base.to_vec();
    for extra in infer::columns_of(docs) {
        if !columns.iter().any(|c| c.name == extra.name) {
            columns.push(extra);
        }
    }
    columns
}

fn column_metas(columns: &[ColumnInfo]) -> Vec<ColumnMeta> {
    columns
        .iter()
        .map(|c| ColumnMeta {
            name: c.name.clone(),
            type_name: c.data_type.clone(),
            family: c.family,
        })
        .collect()
}

/// The structure's columns narrowed to the request's (with `_id` first).
fn requested_columns(request: &BrowseRequest, structure: &TableStructure) -> Vec<ColumnMeta> {
    let all = column_metas(&structure.columns);
    if request.columns.is_empty() {
        return all;
    }
    let mut out: Vec<ColumnMeta> = Vec::new();
    for name in std::iter::once("_id").chain(request.columns.iter().map(String::as_str)) {
        if out.iter().any(|c| c.name == name) {
            continue;
        }
        out.push(
            all.iter()
                .find(|c| c.name == name)
                .cloned()
                .unwrap_or_else(|| ColumnMeta {
                    name: name.to_owned(),
                    type_name: "unknown".into(),
                    family: crate::model::TypeFamily::Other,
                }),
        );
    }
    out
}

fn is_unauthorized(err: &Error) -> bool {
    match err {
        Error::Database { code, message, .. } => {
            matches!(code.as_deref(), Some("13" | "8000"))
                || message.contains("not authorized")
                || message.contains("not allowed to do action")
        }
        _ => false,
    }
}

fn table_kind(kind: &str) -> TableKind {
    match kind {
        "view" => TableKind::View,
        "timeseries" => TableKind::TimeSeries,
        _ => TableKind::Collection,
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

#[async_trait]
impl Driver for MongoDriver {
    fn engine(&self) -> Engine {
        Engine::Mongodb
    }

    fn dialect(&self) -> Dialect {
        Dialect::Mongo
    }

    async fn server_version(&self) -> Result<String> {
        let info = self
            .client
            .database("admin")
            .run_command(doc! { "buildInfo": 1 })
            .await
            .map_err(db_err)?;
        let version = info.get_str("version").unwrap_or("unknown");
        Ok(format!("MongoDB {version}"))
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        let reply = self
            .client
            .database("admin")
            .run_command(doc! {
                "listDatabases": 1,
                "nameOnly": true,
                "authorizedDatabases": true,
            })
            .await
            .map_err(db_err);
        match reply {
            Ok(reply) => {
                let mut names: Vec<String> = reply
                    .get_array("databases")
                    .into_iter()
                    .flatten()
                    .filter_map(|d| d.as_document()?.get_str("name").ok().map(str::to_owned))
                    .collect();
                names.sort();
                Ok(names)
            }
            Err(err) if is_unauthorized(&err) => {
                // Users without the listDatabases privilege still know their own database.
                match &self.configured_db {
                    Some(db) => Ok(vec![db.clone()]),
                    None => Err(err),
                }
            }
            Err(err) => Err(err),
        }
    }

    async fn list_schemas(&self, _database: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn list_tables(&self, database: &str, _schema: Option<&str>) -> Result<Vec<TableInfo>> {
        let entries = self
            .cursor_command(
                database,
                doc! { "listCollections": 1, "nameOnly": true, "authorizedCollections": true },
            )
            .await?;
        let db_name = self.database(database).name().to_owned();
        let named: Vec<(String, TableKind)> = entries
            .iter()
            .filter_map(|e| {
                let name = e.get_str("name").ok()?;
                if name.starts_with("system.") {
                    return None;
                }
                Some((
                    name.to_owned(),
                    table_kind(e.get_str("type").unwrap_or("collection")),
                ))
            })
            .collect();

        let db = self.database(database);
        let mut tables: Vec<TableInfo> = stream::iter(named)
            .map(|(name, kind)| {
                let coll: Collection<Document> = db.collection(&name);
                let db_name = db_name.clone();
                async move {
                    // Views have no cheap count.
                    let estimated_rows = if kind == TableKind::View {
                        None
                    } else {
                        coll.estimated_document_count()
                            .max_time(Duration::from_secs(3))
                            .await
                            .ok()
                    };
                    TableInfo {
                        table: TableRef {
                            database: db_name,
                            schema: None,
                            name,
                        },
                        kind,
                        estimated_rows,
                        comment: None,
                    }
                }
            })
            .buffer_unordered(16)
            .collect()
            .await;
        tables.sort_by(|a, b| a.table.name.cmp(&b.table.name));
        Ok(tables)
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        Ok((*self.structure(table).await?).clone())
    }

    async fn browse_page(&self, request: &BrowseRequest, offset: u64, limit: u32) -> Result<Page> {
        let started = Instant::now();
        let sort = filter::build_sort(request);
        let projection = filter::build_projection(request);

        // Typing filter values needs the structure first; without filters it
        // is fetched alongside the page.
        let (structure, query_filter, docs) = if request.filters.is_empty() {
            let query_filter = filter::build_filter(request, &[])?;
            let page = self.find_page(
                &request.table,
                &query_filter,
                &sort,
                projection.as_ref(),
                offset,
                limit,
            );
            let (structure, docs) = tokio::join!(self.structure(&request.table), page);
            (structure?, query_filter, docs?)
        } else {
            let structure = self.structure(&request.table).await?;
            let query_filter = filter::build_filter(request, &structure.columns)?;
            let docs = self
                .find_page(
                    &request.table,
                    &query_filter,
                    &sort,
                    projection.as_ref(),
                    offset,
                    limit,
                )
                .await?;
            (structure, query_filter, docs)
        };

        // Projected pages only hold the requested fields.
        let base = requested_columns(request, &structure);
        let columns = if request.columns.is_empty() {
            merge_columns(&base, &docs)
        } else {
            base
        };
        let rows = rows_of(&docs, &columns, true);
        Ok(Page {
            columns,
            rows,
            offset,
            query: render::render_find(
                &request.table.name,
                &query_filter,
                projection.as_ref(),
                Some(&sort),
                offset,
                Some(i64::from(limit)),
            ),
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn count_rows(&self, request: &BrowseRequest) -> Result<RowCount> {
        let coll = self.collection(&request.table);
        record(&format!(
            "{}.estimatedDocumentCount()",
            render::collection_ref(&request.table.name)
        ));
        let estimate = coll
            .estimated_document_count()
            .max_time(COUNT_TIMEOUT)
            .await
            .ok();
        let unfiltered = request.filters.is_empty()
            && request
                .raw_where
                .as_deref()
                .is_none_or(|w| w.trim().is_empty());
        if unfiltered && estimate.is_some() {
            return Ok(RowCount {
                exact: None,
                estimate,
            });
        }

        let columns = if request.filters.is_empty() {
            Vec::new()
        } else {
            self.structure(&request.table).await?.columns.clone()
        };
        let query_filter = filter::build_filter(request, &columns)?;
        record(&format!(
            "{}.countDocuments({})",
            render::collection_ref(&request.table.name),
            render::render_doc(&query_filter)
        ));
        match coll
            .count_documents(query_filter)
            .max_time(COUNT_TIMEOUT)
            .await
        {
            Ok(n) => Ok(RowCount {
                exact: Some(n),
                estimate,
            }),
            Err(e) if error_code(&e) == Some(MAX_TIME_EXPIRED) => Ok(RowCount {
                exact: None,
                estimate,
            }),
            Err(e) => Err(db_err(e)),
        }
    }

    async fn browse_all(&self, request: &BrowseRequest) -> Result<Box<dyn Cursor>> {
        let structure = self.structure(&request.table).await?;
        let query_filter = filter::build_filter(request, &structure.columns)?;
        let sort = filter::build_sort(request);
        let projection = filter::build_projection(request);
        let columns = requested_columns(request, &structure);
        let coll = self.collection(&request.table);
        record(&render::render_find(
            &request.table.name,
            &query_filter,
            projection.as_ref(),
            Some(&sort),
            0,
            None,
        ));

        let token = CancellationToken::new();
        let comment = format!("tablesail:{}", uuid::Uuid::new_v4());
        let cancel = MongoCancel::new(self.client.clone(), comment.clone(), token.clone());
        Ok(Box::new(ChannelCursor::spawn(
            cancel,
            move |tx| async move {
                let mut find = coll
                    .find(query_filter)
                    .sort(sort)
                    .comment(Bson::String(comment))
                    .batch_size(1000);
                if let Some(p) = projection {
                    find = find.projection(p);
                }
                let cursor = tokio::select! {
                    _ = token.cancelled() => return Err(Error::Cancelled),
                    r = find => r.map_err(db_err)?,
                };
                exec::emit_docs(Box::pin(cursor), columns, &tx, &token, false).await
            },
        )))
    }

    async fn query(&self, database: Option<&str>, sql: &str) -> Result<Box<dyn Cursor>> {
        let statement = shell::parse(sql)?;
        record(sql.trim());
        let db = statement
            .database
            .clone()
            .or_else(|| {
                database
                    .map(str::trim)
                    .filter(|d| !d.is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| self.default_db.clone());

        let token = CancellationToken::new();
        let comment = format!("tablesail:{}", uuid::Uuid::new_v4());
        let cancel = MongoCancel::new(self.client.clone(), comment.clone(), token.clone());
        let ctx = exec::ExecCtx {
            client: self.client.clone(),
            database: db,
            comment,
            token,
            structures: self.structures.clone(),
        };
        Ok(Box::new(ChannelCursor::spawn(cancel, move |tx| {
            exec::run(ctx, statement.op, tx)
        })))
    }

    async fn preview_changes(&self, changes: &ChangeSet) -> Result<Vec<String>> {
        let structure = self.structure(&changes.table).await?;
        let planned = changes::plan(changes, &structure)?;
        Ok(planned
            .iter()
            .map(|p| p.render(&changes.table.name))
            .collect())
    }

    async fn apply_changes(&self, changes: &ChangeSet) -> Result<ApplyResult> {
        self.apply(changes).await
    }

    async fn insert_rows(
        &self,
        table: &TableRef,
        columns: &[String],
        rows: Vec<Vec<Option<String>>>,
    ) -> Result<u64> {
        self.insert_csv(table, columns, rows).await
    }

    async fn close(&self) {
        self.client.clone().shutdown().await;
    }
}

impl MongoDriver {
    async fn find_page(
        &self,
        table: &TableRef,
        query_filter: &Document,
        sort: &Document,
        projection: Option<&Document>,
        offset: u64,
        limit: u32,
    ) -> Result<Vec<Document>> {
        record(&render::render_find(
            &table.name,
            query_filter,
            projection,
            Some(sort),
            offset,
            Some(i64::from(limit)),
        ));
        let coll = self.collection(table);
        let mut find = coll
            .find(query_filter.clone())
            .sort(sort.clone())
            .skip(offset)
            .limit(i64::from(limit))
            .batch_size(limit.clamp(1, 10_000));
        if let Some(p) = projection {
            find = find.projection(p.clone());
        }
        find.await
            .map_err(db_err)?
            .try_collect()
            .await
            .map_err(db_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionConfig, EnvTag};
    use std::collections::BTreeMap;

    fn params(user: &str, password: Option<&str>, host: &str) -> ConnectParams {
        ConnectParams {
            config: ConnectionConfig {
                id: String::new(),
                name: "t".into(),
                engine: Engine::Mongodb,
                host: host.into(),
                port: 27017,
                user: user.into(),
                database: String::new(),
                file_path: None,
                url: None,
                ssl: SslConfig::default(),
                ssh: None,
                env: EnvTag::Local,
                color: None,
                read_only: false,
                options: BTreeMap::new(),
            },
            password: password.map(str::to_owned),
            host: host.into(),
            port: 27017,
        }
    }

    #[test]
    fn uri_encodes_credentials_and_options() {
        let mut p = params("us@er", Some("p:w/d?#"), "db.example.com");
        p.config.options.insert("authSource".into(), "admin".into());
        let uri = build_uri(&p);
        assert_eq!(
            uri,
            "mongodb://us%40er:p%3Aw%2Fd%3F%23@db.example.com:27017/?authSource=admin&directConnection=true"
        );
        p.config.options.insert("replicaSet".into(), "rs0".into());
        assert!(!build_uri(&p).contains("directConnection"));
        assert!(build_uri(&params("", None, "::1")).starts_with("mongodb://[::1]:27017/"));
    }
}
