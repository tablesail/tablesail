//! Streaming export (cursor → file, never the whole result in memory) and CSV import.
//!
//! Export pulls batches of ~1000 rows from a `Cursor`, formats each batch into a
//! reusable byte buffer and writes it through a buffered file, so memory use is
//! one batch regardless of the table size.
//!
//! Limitation: exporting a *query* goes through `Driver::query`, which applies
//! the grid's truncation (text beyond 16K characters, blobs beyond 4 KiB). The
//! job finishes with a `warning` when any value was cut; export the table itself
//! (`ExportSource::Table`, which streams complete values) to get everything.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};

pub use tokio_util::sync::CancellationToken;

use crate::dialect::Dialect;
use crate::driver::{Cursor, CursorEvent, Driver};
use crate::error::{Error, Result};
use crate::model::{ColumnMeta, CsvImport, CsvPreview, ExportFormat, ExportSource, JobProgress};
use crate::value::{JS_SAFE_INT, Row, Value};

/// Called with progress at most every ~100 ms and once at the end.
pub type ProgressFn = Arc<dyn Fn(&JobProgress) + Send + Sync>;

const BATCH: usize = 1000;
const REPORT_EVERY: Duration = Duration::from_millis(100);

pub(crate) fn progress_of(rows: u64, bytes: u64) -> JobProgress {
    JobProgress {
        rows,
        bytes,
        done: false,
        cancelled: false,
        error: None,
        warning: None,
    }
}

/// Rate-limits progress callbacks.
pub(crate) struct Reporter {
    progress: ProgressFn,
    last: Instant,
}

impl Reporter {
    pub(crate) fn new(progress: ProgressFn) -> Self {
        Reporter {
            progress,
            last: Instant::now(),
        }
    }

    pub(crate) fn tick(&mut self, rows: u64, bytes: u64) {
        if self.last.elapsed() >= REPORT_EVERY {
            self.last = Instant::now();
            (self.progress)(&progress_of(rows, bytes));
        }
    }

    pub(crate) fn finish(&self, p: &JobProgress) {
        (self.progress)(p);
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Streams `source` to `path` in `format`. Cancelling leaves a partial file and
/// returns progress with `cancelled: true`.
pub async fn export(
    driver: Arc<dyn Driver>,
    source: ExportSource,
    format: ExportFormat,
    path: PathBuf,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<JobProgress> {
    let dialect = driver.dialect();
    let formatter = Formatter::new(&format, dialect)?;
    // Opening can take a while (a slow query returns its cursor only once it starts
    // producing), so cancellation must be honoured here too.
    let open = async {
        match &source {
            ExportSource::Table { request } => driver.browse_all(request).await.map(|c| (c, false)),
            ExportSource::Query { database, sql } => driver
                .query(database.as_deref(), sql)
                .await
                .map(|c| (c, true)),
        }
    };
    let (cursor, from_query) = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let done = JobProgress { done: true, cancelled: true, ..progress_of(0, 0) };
            progress(&done);
            return Ok(done);
        }
        opened = open => opened?,
    };
    let file = tokio::fs::File::create(&path)
        .await
        .map_err(|e| Error::Io(format!("cannot create {}: {e}", path.display())))?;
    let out = BufWriter::with_capacity(256 * 1024, file);
    run_export(cursor, formatter, from_query, out, progress, cancel).await
}

async fn run_export<W: AsyncWrite + Unpin>(
    mut cursor: Box<dyn Cursor>,
    mut formatter: Formatter,
    from_query: bool,
    mut out: W,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<JobProgress> {
    let mut reporter = Reporter::new(progress);
    let cancel_handle = cursor.cancel_handle();
    let mut buf: Vec<u8> = Vec::with_capacity(256 * 1024);
    let (mut rows_done, mut bytes_done) = (0u64, 0u64);
    let mut truncated_values = 0u64;
    let mut cancelled = false;
    let mut extra_sets = false;
    let mut started = false;

    let outcome: Result<()> = async {
        loop {
            let event = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    cancelled = true;
                    break;
                }
                event = cursor.next(BATCH) => event?,
            };
            match event {
                None => break,
                Some(CursorEvent::Columns(columns)) => {
                    if started {
                        // Only the first result set is exported (MySQL CALL may return several).
                        extra_sets = true;
                        break;
                    }
                    started = true;
                    formatter.begin(&columns, &mut buf)?;
                }
                Some(CursorEvent::Rows(rows)) => {
                    if !started {
                        started = true;
                        formatter.begin(&[], &mut buf)?;
                    }
                    if from_query {
                        truncated_values +=
                            rows.iter().flatten().filter(|v| !v.is_complete()).count() as u64;
                    }
                    formatter.rows(&rows, &mut buf)?;
                    rows_done += rows.len() as u64;
                    flush(&mut out, &mut buf, &mut bytes_done).await?;
                    reporter.tick(rows_done, bytes_done);
                }
                Some(CursorEvent::Affected(_)) => {}
            }
        }
        if cancelled {
            return Ok(());
        }
        if !started {
            formatter.begin(&[], &mut buf)?;
        }
        formatter.finish(&mut buf)?;
        flush(&mut out, &mut buf, &mut bytes_done).await?;
        out.flush().await?;
        Ok(())
    }
    .await;

    if cancelled {
        // Stop the statement on the server too, then keep what was written.
        let _ = cancel_handle.cancel().await;
        let _ = out.flush().await;
        let done = JobProgress {
            done: true,
            cancelled: true,
            ..progress_of(rows_done, bytes_done)
        };
        reporter.finish(&done);
        return Ok(done);
    }

    if let Err(e) = outcome {
        let _ = out.flush().await;
        reporter.finish(&JobProgress {
            done: true,
            error: Some(e.to_string()),
            ..progress_of(rows_done, bytes_done)
        });
        return Err(e);
    }

    let mut notes = Vec::new();
    if truncated_values > 0 {
        notes.push(format!(
            "{truncated_values} long text or binary value(s) were cut short because the export \
             ran as a query. Export the table itself to get complete values."
        ));
    }
    if extra_sets {
        notes.push(
            "The statement returned several result sets; only the first was exported.".into(),
        );
    }
    let done = JobProgress {
        done: true,
        warning: (!notes.is_empty()).then(|| notes.join(" ")),
        ..progress_of(rows_done, bytes_done)
    };
    reporter.finish(&done);
    Ok(done)
}

async fn flush<W: AsyncWrite + Unpin>(
    out: &mut W,
    buf: &mut Vec<u8>,
    bytes: &mut u64,
) -> Result<()> {
    if !buf.is_empty() {
        out.write_all(buf).await?;
        *bytes += buf.len() as u64;
        buf.clear();
    }
    Ok(())
}

/// Encodes a batch of rows into bytes for one output format.
enum Formatter {
    Csv {
        delimiter: u8,
        header: bool,
        null_as: String,
        /// MySQL and SQLite store booleans as 0/1 and cannot read `true`/`false` back
        /// (SQLite would keep the word as text), so those are written as numbers.
        numeric_bools: bool,
    },
    Json {
        pretty: bool,
        columns: Vec<String>,
        first: bool,
    },
    JsonLines {
        columns: Vec<String>,
    },
    Sql(SqlWriter),
}

type LiteralFn = Box<dyn Fn(&Value) -> Result<String> + Send>;

struct SqlWriter {
    dialect: Dialect,
    literal: LiteralFn,
    table: String,
    rows_per_insert: u32,
    prefix: String,
    in_statement: u32,
}

impl Formatter {
    fn new(format: &ExportFormat, dialect: Dialect) -> Result<Self> {
        Ok(match format {
            ExportFormat::Csv {
                delimiter,
                header,
                null_as,
            } => {
                let delimiter = u8::try_from(*delimiter)
                    .ok()
                    .filter(|b| b.is_ascii() && *b != b'"' && *b != b'\n' && *b != b'\r')
                    .ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "'{delimiter}' cannot be used as a CSV delimiter."
                        ))
                    })?;
                Formatter::Csv {
                    delimiter,
                    header: *header,
                    null_as: null_as.clone(),
                    numeric_bools: matches!(dialect, Dialect::Mysql { .. } | Dialect::Sqlite),
                }
            }
            ExportFormat::Json { pretty } => Formatter::Json {
                pretty: *pretty,
                columns: Vec::new(),
                first: true,
            },
            ExportFormat::JsonLines => Formatter::JsonLines {
                columns: Vec::new(),
            },
            ExportFormat::Sql {
                table,
                rows_per_insert,
            } => {
                if matches!(dialect, Dialect::Mongo) {
                    return Err(Error::Unsupported(
                        "SQL export is not available for MongoDB; use JSON instead.".into(),
                    ));
                }
                if table.trim().is_empty() {
                    return Err(Error::InvalidInput(
                        "A table name is required for SQL export.".into(),
                    ));
                }
                Formatter::Sql(SqlWriter {
                    dialect,
                    literal: Box::new(move |v| dialect.literal(v)),
                    table: dialect.quote_ident(table),
                    rows_per_insert: (*rows_per_insert).max(1),
                    prefix: String::new(),
                    in_statement: 0,
                })
            }
        })
    }

    fn begin(&mut self, columns: &[ColumnMeta], buf: &mut Vec<u8>) -> Result<()> {
        match self {
            Formatter::Csv {
                delimiter, header, ..
            } => {
                if *header && !columns.is_empty() {
                    write_csv(*delimiter, buf, |w| {
                        w.write_record(columns.iter().map(|c| c.name.as_str()))
                    })?;
                }
            }
            Formatter::Json {
                columns: names,
                pretty,
                ..
            } => {
                *names = columns.iter().map(|c| c.name.clone()).collect();
                buf.extend_from_slice(if *pretty { b"[" } else { b"[\n" });
            }
            Formatter::JsonLines { columns: names } => {
                *names = columns.iter().map(|c| c.name.clone()).collect();
            }
            Formatter::Sql(sql) => {
                let cols: Vec<String> = columns
                    .iter()
                    .map(|c| sql.dialect.quote_ident(&c.name))
                    .collect();
                sql.prefix = format!("INSERT INTO {} ({}) VALUES\n", sql.table, cols.join(", "));
            }
        }
        Ok(())
    }

    fn rows(&mut self, rows: &[Row], buf: &mut Vec<u8>) -> Result<()> {
        match self {
            Formatter::Csv {
                delimiter,
                null_as,
                numeric_bools,
                ..
            } => {
                write_csv(*delimiter, buf, |w| {
                    for row in rows {
                        w.write_record(row.iter().map(|v| match v {
                            Value::Null => null_as.clone(),
                            absent if is_absent(absent) => null_as.clone(),
                            Value::Bool(b) if *numeric_bools => u8::from(*b).to_string(),
                            other => other.to_display(),
                        }))?;
                    }
                    Ok(())
                })?;
            }
            Formatter::Json {
                pretty,
                columns,
                first,
            } => {
                for row in rows {
                    if *pretty {
                        buf.extend_from_slice(if *first { b"\n" } else { b",\n" });
                        write_object_pretty(buf, columns, row, "  ");
                    } else {
                        if !*first {
                            buf.extend_from_slice(b",\n");
                        }
                        write_object(buf, columns, row);
                    }
                    *first = false;
                }
            }
            Formatter::JsonLines { columns } => {
                for row in rows {
                    write_object(buf, columns, row);
                    buf.push(b'\n');
                }
            }
            Formatter::Sql(sql) => {
                for row in rows {
                    let mut text = String::new();
                    if sql.in_statement == 0 {
                        text.push_str(&sql.prefix);
                    } else {
                        text.push_str(",\n");
                    }
                    text.push('(');
                    for (i, value) in row.iter().enumerate() {
                        if i > 0 {
                            text.push_str(", ");
                        }
                        text.push_str(&(sql.literal)(value)?);
                    }
                    text.push(')');
                    sql.in_statement += 1;
                    if sql.in_statement >= sql.rows_per_insert {
                        text.push_str(";\n");
                        sql.in_statement = 0;
                    }
                    buf.extend_from_slice(text.as_bytes());
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self, buf: &mut Vec<u8>) -> Result<()> {
        match self {
            Formatter::Csv { .. } => {}
            Formatter::Json { first, .. } => {
                buf.extend_from_slice(if *first { b"]\n" } else { b"\n]\n" });
            }
            Formatter::JsonLines { .. } => {}
            Formatter::Sql(sql) => {
                if sql.in_statement > 0 {
                    buf.extend_from_slice(b";\n");
                    sql.in_statement = 0;
                }
            }
        }
        Ok(())
    }
}

/// Appends CSV records to `buf` through a short-lived writer (one per batch).
fn write_csv<F>(delimiter: u8, buf: &mut Vec<u8>, write: F) -> Result<()>
where
    F: FnOnce(&mut csv::Writer<&mut Vec<u8>>) -> std::result::Result<(), csv::Error>,
{
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_writer(buf);
    write(&mut writer).map_err(|e| Error::Io(format!("writing CSV: {e}")))?;
    writer.flush()?;
    Ok(())
}

/// A MongoDB field the document does not have. Exports have fixed columns, so it
/// is written like NULL rather than as the empty text it displays as.
fn is_absent(value: &Value) -> bool {
    matches!(value, Value::Other { type_name, .. } if type_name == "missing")
}

/// A value as plain JSON: numbers stay numbers, anything JavaScript cannot hold
/// exactly (big integers, decimals) becomes a string, bytes become base64.
fn write_value(buf: &mut Vec<u8>, value: &Value) {
    let result = match value {
        Value::Null => serde_json::to_writer(&mut *buf, &serde_json::Value::Null),
        absent if is_absent(absent) => serde_json::to_writer(&mut *buf, &serde_json::Value::Null),
        Value::Bool(b) => serde_json::to_writer(&mut *buf, b),
        Value::Int(i) if (-JS_SAFE_INT..=JS_SAFE_INT).contains(i) => {
            serde_json::to_writer(&mut *buf, i)
        }
        Value::Int(i) => serde_json::to_writer(&mut *buf, &i.to_string()),
        Value::UInt(u) if *u <= JS_SAFE_INT as u64 => serde_json::to_writer(&mut *buf, u),
        Value::UInt(u) => serde_json::to_writer(&mut *buf, &u.to_string()),
        Value::Float(f) if f.is_finite() => serde_json::to_writer(&mut *buf, f),
        Value::Float(f) => serde_json::to_writer(&mut *buf, &f.to_string()),
        Value::Json(j) => serde_json::to_writer(&mut *buf, j),
        Value::Bytes { data, .. } => serde_json::to_writer(
            &mut *buf,
            &base64::engine::general_purpose::STANDARD.encode(data),
        ),
        other => serde_json::to_writer(&mut *buf, &other.to_display()),
    };
    // Writing into a Vec cannot fail and every value above is serializable.
    debug_assert!(result.is_ok());
}

fn write_key(buf: &mut Vec<u8>, name: &str) {
    let _ = serde_json::to_writer(&mut *buf, name);
}

fn column_name(columns: &[String], index: usize) -> String {
    columns
        .get(index)
        .cloned()
        .unwrap_or_else(|| format!("column{}", index + 1))
}

fn write_object(buf: &mut Vec<u8>, columns: &[String], row: &Row) {
    buf.push(b'{');
    for (i, value) in row.iter().enumerate() {
        if i > 0 {
            buf.push(b',');
        }
        write_key(buf, &column_name(columns, i));
        buf.push(b':');
        write_value(buf, value);
    }
    buf.push(b'}');
}

fn write_object_pretty(buf: &mut Vec<u8>, columns: &[String], row: &Row, indent: &str) {
    buf.extend_from_slice(indent.as_bytes());
    buf.extend_from_slice(b"{");
    for (i, value) in row.iter().enumerate() {
        buf.extend_from_slice(if i > 0 { b",\n" } else { b"\n" });
        buf.extend_from_slice(indent.as_bytes());
        buf.extend_from_slice(b"  ");
        write_key(buf, &column_name(columns, i));
        buf.extend_from_slice(b": ");
        match value {
            // Nested documents get indented too, which keeps big Mongo exports readable.
            Value::Json(j) => {
                let pretty = serde_json::to_string_pretty(j).unwrap_or_else(|_| "null".into());
                let pad = format!("{indent}  ");
                for (n, line) in pretty.lines().enumerate() {
                    if n > 0 {
                        buf.push(b'\n');
                        buf.extend_from_slice(pad.as_bytes());
                    }
                    buf.extend_from_slice(line.as_bytes());
                }
            }
            other => write_value(buf, other),
        }
    }
    if !row.is_empty() {
        buf.push(b'\n');
        buf.extend_from_slice(indent.as_bytes());
    }
    buf.push(b'}');
}

// ---------------------------------------------------------------------------
// CSV preview and import
// ---------------------------------------------------------------------------

const DELIMITERS: &[u8] = b",;\t|";

fn ascii_delimiter(c: char) -> Result<u8> {
    u8::try_from(c)
        .ok()
        .filter(|b| b.is_ascii() && *b != b'"' && *b != b'\n' && *b != b'\r')
        .ok_or_else(|| Error::InvalidInput(format!("'{c}' cannot be used as a CSV delimiter.")))
}

fn open_csv(path: &Path, delimiter: u8) -> Result<csv::Reader<std::fs::File>> {
    let file = std::fs::File::open(path)
        .map_err(|e| Error::Io(format!("cannot open {}: {e}", path.display())))?;
    Ok(csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(file))
}

fn field_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn strip_bom(mut fields: Vec<String>) -> Vec<String> {
    if let Some(first) = fields.first_mut()
        && let Some(rest) = first.strip_prefix('\u{feff}')
    {
        *first = rest.to_owned();
    }
    fields
}

/// Picks the delimiter whose parse gives the most records with the same, plural, field count.
fn sniff_delimiter(sample: &[u8]) -> u8 {
    let mut best = (b',', 0usize, 0usize);
    for &candidate in DELIMITERS {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(candidate)
            .has_headers(false)
            .flexible(true)
            .from_reader(sample);
        let mut counts: Vec<usize> = Vec::new();
        for record in reader.byte_records().take(50) {
            match record {
                Ok(r) => counts.push(r.len()),
                Err(_) => break,
            }
        }
        // The last record of a sample cut mid-line is unreliable.
        if counts.len() > 2 {
            counts.pop();
        }
        let Some(&modal) = counts
            .iter()
            .max_by_key(|n| counts.iter().filter(|m| m == n).count())
        else {
            continue;
        };
        if modal < 2 {
            continue;
        }
        let agreeing = counts.iter().filter(|&&n| n == modal).count();
        if (agreeing, modal) > (best.1, best.2) {
            best = (candidate, agreeing, modal);
        }
    }
    best.0
}

fn sniff_file(path: &Path) -> Result<u8> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|e| Error::Io(format!("cannot open {}: {e}", path.display())))?;
    let mut sample = vec![0u8; 64 * 1024];
    let n = file.read(&mut sample)?;
    sample.truncate(n);
    Ok(sniff_delimiter(&sample))
}

/// Reads the first rows of a CSV file; `delimiter` None means sniff (`,` `;` tab `|`).
pub fn csv_preview(path: &str, delimiter: Option<char>, rows: usize) -> Result<CsvPreview> {
    let path = Path::new(path);
    let delimiter = match delimiter {
        Some(c) => ascii_delimiter(c)?,
        None => sniff_file(path)?,
    };
    let mut reader = open_csv(path, delimiter)?;
    let mut records = reader.byte_records();
    let headers = match records.next() {
        Some(record) => {
            let record = record.map_err(|e| Error::InvalidInput(format!("reading CSV: {e}")))?;
            strip_bom(record.iter().map(field_text).collect())
        }
        None => Vec::new(),
    };
    let mut out = Vec::with_capacity(rows);
    for record in records.take(rows) {
        let record = record.map_err(|e| Error::InvalidInput(format!("reading CSV: {e}")))?;
        out.push(record.iter().map(field_text).collect());
    }
    Ok(CsvPreview {
        headers,
        rows: out,
        delimiter: delimiter as char,
    })
}

/// Imports a CSV file in batches through `Driver::insert_rows`, in one transaction per batch.
pub async fn import_csv(
    driver: Arc<dyn Driver>,
    import: CsvImport,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<JobProgress> {
    let mut reporter = Reporter::new(progress);
    let (mut rows_done, mut bytes_done) = (0u64, 0u64);

    let outcome: Result<bool> = async {
        let delimiter = ascii_delimiter(import.delimiter)?;
        let targets: Vec<(usize, String)> = import
            .mapping
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.as_ref().filter(|t| !t.is_empty()).map(|t| (i, t.clone())))
            .collect();
        if targets.is_empty() {
            return Err(Error::InvalidInput(
                "Map at least one CSV column to a table column.".into(),
            ));
        }
        let columns: Vec<String> = targets.iter().map(|(_, t)| t.clone()).collect();

        let mut reader = open_csv(Path::new(&import.path), delimiter)?;
        let mut records = reader.byte_records();
        if import.has_header {
            let _ = records.next();
        }
        let mut batch: Vec<Vec<Option<String>>> = Vec::with_capacity(BATCH);
        let mut exhausted = false;
        while !exhausted {
            if cancel.is_cancelled() {
                return Ok(true);
            }
            batch.clear();
            while batch.len() < BATCH {
                match records.next() {
                    None => {
                        exhausted = true;
                        break;
                    }
                    Some(Err(e)) => {
                        let line = e.position().map(|p| p.line()).unwrap_or(0);
                        return Err(Error::InvalidInput(format!("CSV line {line}: {e}")));
                    }
                    Some(Ok(record)) => {
                        let row = targets
                            .iter()
                            .map(|(i, _)| {
                                let field = record.get(*i).map(field_text)?;
                                match &import.null_as {
                                    Some(null) if &field == null => None,
                                    _ => Some(field),
                                }
                            })
                            .collect();
                        batch.push(row);
                    }
                }
            }
            if batch.is_empty() {
                break;
            }
            let position = records.reader().position().byte();
            let rows = std::mem::replace(&mut batch, Vec::with_capacity(BATCH));
            let inserted = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(true),
                result = driver.insert_rows(&import.table, &columns, rows) => result?,
            };
            rows_done += inserted;
            bytes_done = position;
            reporter.tick(rows_done, bytes_done);
        }
        Ok(false)
    }
    .await;

    match outcome {
        Ok(cancelled) => {
            let done = JobProgress {
                done: true,
                cancelled,
                ..progress_of(rows_done, bytes_done)
            };
            reporter.finish(&done);
            Ok(done)
        }
        Err(e) => {
            let mut message = e.to_string();
            if rows_done > 0 {
                message.push_str(&format!(" ({rows_done} rows were already imported.)"));
            }
            reporter.finish(&JobProgress {
                done: true,
                error: Some(message),
                ..progress_of(rows_done, bytes_done)
            });
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::VecCursor;
    use crate::model::TypeFamily;
    use std::sync::Mutex;

    fn col(name: &str) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            type_name: "text".into(),
            family: TypeFamily::Text,
        }
    }

    fn sample_cursor() -> Box<dyn Cursor> {
        Box::new(VecCursor::new(vec![
            CursorEvent::Columns(vec![col("id"), col("name"), col("meta")]),
            CursorEvent::Rows(vec![
                vec![
                    Value::Int(1),
                    Value::Text("a,b \"q\"".into()),
                    Value::Json(serde_json::json!({"k": [1, 2]})),
                ],
                vec![
                    Value::Int(9_007_199_254_740_993),
                    Value::Null,
                    Value::Decimal("1.50".into()),
                ],
                vec![
                    Value::Bool(true),
                    Value::bytes(vec![0xde, 0xad]),
                    Value::DateTime("2024-01-02 03:04:05".into()),
                ],
            ]),
        ]))
    }

    async fn run(format: ExportFormat) -> (String, JobProgress) {
        let formatter = Formatter::new(&format, Dialect::Sqlite).unwrap();
        let mut out = Vec::new();
        let seen = Arc::new(Mutex::new(0usize));
        let seen2 = seen.clone();
        let progress: ProgressFn = Arc::new(move |_| *seen2.lock().unwrap() += 1);
        let done = run_export(
            sample_cursor(),
            formatter,
            false,
            &mut out,
            progress,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(
            *seen.lock().unwrap() >= 1,
            "final progress is always reported"
        );
        (String::from_utf8(out).unwrap(), done)
    }

    #[tokio::test]
    async fn csv_quotes_and_nulls() {
        let (text, done) = run(ExportFormat::Csv {
            delimiter: ';',
            header: true,
            null_as: "NULL".into(),
        })
        .await;
        assert_eq!(
            text,
            "id;name;meta\n1;\"a,b \"\"q\"\"\";\"{\"\"k\"\":[1,2]}\"\n\
             9007199254740993;NULL;1.50\n1;0xdead;2024-01-02 03:04:05\n"
        );
        assert_eq!(done.rows, 3);
        assert_eq!(done.bytes, text.len() as u64);
        assert!(done.done && !done.cancelled && done.error.is_none());
    }

    #[tokio::test]
    async fn csv_booleans_are_numbers_where_the_database_stores_numbers() {
        for (dialect, t, f) in [
            (Dialect::Sqlite, "1", "0"),
            (
                Dialect::Mysql {
                    backslash_escapes: true,
                },
                "1",
                "0",
            ),
            (Dialect::Postgres, "true", "false"),
        ] {
            let format = ExportFormat::Csv {
                delimiter: ',',
                header: false,
                null_as: String::new(),
            };
            let mut formatter = Formatter::new(&format, dialect).unwrap();
            let mut buf = Vec::new();
            formatter.begin(&[col("a"), col("b")], &mut buf).unwrap();
            formatter
                .rows(&[vec![Value::Bool(true), Value::Bool(false)]], &mut buf)
                .unwrap();
            assert_eq!(String::from_utf8(buf).unwrap(), format!("{t},{f}\n"));
        }
    }

    #[tokio::test]
    async fn absent_mongo_fields_export_like_null() {
        let absent = Value::Other {
            type_name: "missing".into(),
            text: String::new(),
        };
        let cursor = || -> Box<dyn Cursor> {
            Box::new(VecCursor::new(vec![
                CursorEvent::Columns(vec![col("a"), col("b")]),
                CursorEvent::Rows(vec![vec![absent.clone(), Value::Text(String::new())]]),
            ]))
        };
        for (format, expected) in [
            (
                ExportFormat::JsonLines,
                "{\"a\":null,\"b\":\"\"}\n".to_owned(),
            ),
            (
                ExportFormat::Csv {
                    delimiter: ',',
                    header: false,
                    null_as: "NULL".into(),
                },
                "NULL,\n".to_owned(),
            ),
        ] {
            let formatter = Formatter::new(&format, Dialect::Mongo).unwrap();
            let mut out = Vec::new();
            let progress: ProgressFn = Arc::new(|_| {});
            run_export(
                cursor(),
                formatter,
                false,
                &mut out,
                progress,
                CancellationToken::new(),
            )
            .await
            .unwrap();
            assert_eq!(String::from_utf8(out).unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn json_lines_use_plain_json() {
        let (text, _) = run(ExportFormat::JsonLines).await;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            r#"{"id":1,"name":"a,b \"q\"","meta":{"k":[1,2]}}"#
        );
        assert_eq!(
            lines[1],
            r#"{"id":"9007199254740993","name":null,"meta":"1.50"}"#
        );
        assert_eq!(
            lines[2],
            r#"{"id":true,"name":"3q0=","meta":"2024-01-02 03:04:05"}"#
        );
    }

    #[tokio::test]
    async fn json_array_parses_back() {
        for pretty in [false, true] {
            let (text, _) = run(ExportFormat::Json { pretty }).await;
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
            let array = parsed.as_array().unwrap();
            assert_eq!(array.len(), 3, "{text}");
            assert_eq!(array[0]["meta"]["k"][1], 2);
            assert_eq!(array[1]["id"], "9007199254740993");
            // Column order survives.
            let keys: Vec<&String> = array[0].as_object().unwrap().keys().collect();
            assert_eq!(keys.len(), 3);
        }
    }

    #[tokio::test]
    async fn empty_json_export_is_an_empty_array() {
        let formatter =
            Formatter::new(&ExportFormat::Json { pretty: true }, Dialect::Sqlite).unwrap();
        let mut out = Vec::new();
        let progress: ProgressFn = Arc::new(|_| {});
        run_export(
            Box::new(VecCursor::new(vec![CursorEvent::Columns(vec![col("a")])])),
            formatter,
            false,
            &mut out,
            progress,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed, serde_json::json!([]));
    }

    #[test]
    fn sql_writer_batches_rows() {
        let mut formatter = Formatter::Sql(SqlWriter {
            dialect: Dialect::Postgres,
            literal: Box::new(|v| Ok(format!("<{}>", v.to_display()))),
            table: Dialect::Postgres.quote_ident("t"),
            rows_per_insert: 2,
            prefix: String::new(),
            in_statement: 0,
        });
        let mut buf = Vec::new();
        formatter.begin(&[col("a"), col("b")], &mut buf).unwrap();
        let rows: Vec<Row> = (1..=3)
            .map(|i| vec![Value::Int(i), Value::Text(format!("x{i}"))])
            .collect();
        formatter.rows(&rows, &mut buf).unwrap();
        formatter.finish(&mut buf).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "INSERT INTO \"t\" (\"a\", \"b\") VALUES\n(<1>, <x1>),\n(<2>, <x2>);\n\
             INSERT INTO \"t\" (\"a\", \"b\") VALUES\n(<3>, <x3>);\n"
        );
    }

    #[tokio::test]
    async fn cancelled_export_reports_cancelled() {
        let formatter = Formatter::new(&ExportFormat::JsonLines, Dialect::Sqlite).unwrap();
        let mut out = Vec::new();
        let progress: ProgressFn = Arc::new(|_| {});
        let cancel = CancellationToken::new();
        cancel.cancel();
        let done = run_export(
            sample_cursor(),
            formatter,
            false,
            &mut out,
            progress,
            cancel,
        )
        .await
        .unwrap();
        assert!(done.cancelled && done.done);
    }

    #[tokio::test]
    async fn query_export_warns_about_truncated_values() {
        let cursor = Box::new(VecCursor::new(vec![
            CursorEvent::Columns(vec![col("t")]),
            CursorEvent::Rows(vec![vec![Value::TruncatedText {
                prefix: "abc".into(),
                len: 100_000,
            }]]),
        ]));
        let formatter = Formatter::new(&ExportFormat::JsonLines, Dialect::Sqlite).unwrap();
        let mut out = Vec::new();
        let progress: ProgressFn = Arc::new(|_| {});
        let done = run_export(
            cursor,
            formatter,
            true,
            &mut out,
            progress,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(done.warning.unwrap().contains("cut short"));
    }

    #[test]
    fn sniffs_delimiters() {
        assert_eq!(sniff_delimiter(b"a,b,c\n1,2,3\n4,5,6\n"), b',');
        assert_eq!(sniff_delimiter(b"a;b;c\n1;2;3\n4;5;6\n"), b';');
        assert_eq!(sniff_delimiter(b"a\tb\tc\n1\t2\t3\n4\t5\t6\n"), b'\t');
        assert_eq!(sniff_delimiter(b"a|b\n1|2\n3|4\n"), b'|');
        // Commas inside a single column do not fool the sniffer.
        assert_eq!(sniff_delimiter(b"a;b\n\"x,y,z\";2\n\"p,q,r\";4\n"), b';');
        assert_eq!(sniff_delimiter(b"single\nvalues\n"), b',');
    }

    #[test]
    fn previews_a_file() {
        let dir = std::env::temp_dir().join(format!("tablesail-csv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.csv");
        std::fs::write(&path, "\u{feff}id;name\n1;ada\n2;\"lin;us\"\n3;grace\n").unwrap();
        let preview = csv_preview(path.to_str().unwrap(), None, 2).unwrap();
        assert_eq!(preview.delimiter, ';');
        assert_eq!(preview.headers, vec!["id", "name"]);
        assert_eq!(preview.rows, vec![vec!["1", "ada"], vec!["2", "lin;us"]]);
        std::fs::remove_dir_all(dir).ok();
    }
}
