//! Runs a .sql file (usually a dump) against a connection, statement by statement.
//!
//! The file is read in pieces and split with [`split_chunk`], so a dump of any size needs only a
//! few megabytes of memory: a piece's complete statements are run, the unfinished rest is kept
//! and completed by the next piece. Each statement goes through `Driver::query` and its cursor is
//! drained, which also puts it in the query log. The whole file runs on one connection of its own
//! (`ScriptConnection::dedicated`), so `SET`, `USE`, `BEGIN` and temporary tables stay in effect
//! for the statements after them, whatever else the session is doing meanwhile.
//!
//! What a dump of each engine needs beyond plain statements:
//! * MySQL: `DELIMITER`, handled by the splitter.
//! * PostgreSQL: pg_dump writes psql meta-commands (`\restrict`, `\connect`) and loads data with
//!   `COPY ... FROM stdin` blocks. The harmless meta-commands are skipped; `COPY` blocks cannot
//!   be run through a plain connection, so they fail with advice to dump with `--inserts`.
//! * SQLite: `.dump` wraps everything in `BEGIN ... COMMIT`; a transaction the file leaves open
//!   is rolled back instead of lingering on the connection.

use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use regex::Regex;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::Engine;
use crate::dialect::Dialect;
use crate::driver::Driver;
use crate::error::{Error, READ_ONLY, Result};
use crate::export::{CancellationToken, ProgressFn, Reporter, progress_of};
use crate::model::{JobProgress, SqlImport, StatementKind, StatementSpan};
use crate::split::{ChunkState, split_chunk, statement_kind};
use crate::transaction::ScriptConnection;

/// Bytes read from the file at a time.
const CHUNK: usize = 4 * 1024 * 1024;
/// Rows requested per call while draining a statement's cursor.
const BATCH: usize = 1000;
/// Longest the server-side cancel of a running statement may take.
const CANCEL_WAIT: Duration = Duration::from_secs(5);

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

const COPY_STDIN: &str = "COPY ... FROM stdin blocks can't be imported. Export the dump again with \
                          pg_dump --inserts (or --column-inserts), or load it with psql.";

/// The problems that can be reported before the job starts: MongoDB, no file, a file that
/// can't be read. (`run_sql_import` checks again, the caller may want the answer at once.)
pub async fn check(engine: Engine, import: &SqlImport) -> Result<()> {
    if engine == Engine::Mongodb {
        return Err(Error::InvalidInput(
            "SQL files can't be imported into MongoDB. Use the shell editor instead.".to_owned(),
        ));
    }
    if import.path.trim().is_empty() {
        return Err(Error::InvalidInput(
            "Choose a .sql file to import.".to_owned(),
        ));
    }
    let path = Path::new(&import.path);
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| Error::Io(format!("cannot read {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::InvalidInput(format!(
            "{} is not a file.",
            path.display()
        )));
    }
    Ok(())
}

/// Runs the file named by `import` against `driver`, on a connection of its own that is closed
/// again when the job ends (a transaction the file left open goes with it).
///
/// Progress: `rows` counts the statements that ran without error, `bytes` the part of the file
/// consumed. A statement that fails ends the job with the error `Statement N failed: <message>`
/// when `stop_on_error` is set; otherwise the job carries on and finishes with a warning
/// `K of N statements failed; first: ...`. Cancelling stops between statements and interrupts
/// the running one. On a `read_only` connection only reading statements are accepted.
///
/// `Err` is for problems before the first statement (no file, MongoDB); later problems end the
/// job through its progress.
pub async fn run_sql_import(
    driver: Arc<dyn Driver>,
    import: SqlImport,
    read_only: bool,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<JobProgress> {
    let dialect = driver.dialect();
    let own_connection = true;
    run_chunked(
        driver,
        dialect,
        import,
        read_only,
        progress,
        cancel,
        CHUNK,
        own_connection,
    )
    .await
}

/// `run_sql_import` with the piece size and dialect chosen by the caller (tests); without
/// `own_connection` the statements go to `driver` itself.
#[allow(clippy::too_many_arguments)]
async fn run_chunked(
    driver: Arc<dyn Driver>,
    dialect: Dialect,
    import: SqlImport,
    read_only: bool,
    progress: ProgressFn,
    cancel: CancellationToken,
    chunk: usize,
    own_connection: bool,
) -> Result<JobProgress> {
    check(driver.engine(), &import).await?;
    let path = Path::new(&import.path);
    let file = File::open(path)
        .await
        .map_err(|e| Error::Io(format!("cannot read {}: {e}", path.display())))?;

    let database = import.database.filter(|d| !d.trim().is_empty());
    let script = if own_connection {
        Some(ScriptConnection::dedicated(&driver, database.as_deref()).await?)
    } else {
        None
    };
    let mut importer = Importer {
        driver: script
            .as_ref()
            .map_or_else(|| driver.clone(), |s| s.driver().clone()),
        script,
        dialect,
        database,
        read_only,
        stop_on_error: import.stop_on_error,
        cancel,
        reporter: Reporter::new(progress),
        attempted: 0,
        ran: 0,
        failed: 0,
        first_failure: None,
        bytes: 0,
        open_transaction: false,
    };
    let finish = match importer.read_and_run(file, path, chunk).await {
        Ok(finish) => finish,
        Err(e) => Finish::Failed(e.to_string()),
    };
    Ok(importer.conclude(finish).await)
}

enum Finish {
    /// The whole file was read.
    Completed,
    Cancelled,
    /// A statement failed and `stop_on_error` is set (or the failure cannot be skipped).
    Stopped(String),
    /// The file could not be read.
    Failed(String),
}

enum Flow {
    Next,
    /// The statement was a `COPY ... FROM stdin`; its data lines follow and must be skipped.
    SkipCopyData,
    Stop(String),
    Cancelled,
}

struct Importer {
    /// Where the statements run: the connection of `script`.
    driver: Arc<dyn Driver>,
    /// The connection of its own, closed when the job ends.
    script: Option<ScriptConnection>,
    dialect: Dialect,
    database: Option<String>,
    read_only: bool,
    stop_on_error: bool,
    cancel: CancellationToken,
    reporter: Reporter,
    /// Statements started, ran fine, and failed (a failure that stops the job is not counted).
    attempted: u64,
    ran: u64,
    failed: u64,
    /// "statement 12: <message>" of the first failure that was skipped.
    first_failure: Option<String>,
    /// Bytes of the file consumed by finished statements.
    bytes: u64,
    /// The file ran a BEGIN that no COMMIT/ROLLBACK has closed yet.
    open_transaction: bool,
}

impl Importer {
    async fn read_and_run(&mut self, mut file: File, path: &Path, chunk: usize) -> Result<Finish> {
        let chunk = chunk.max(1);
        let mut buf: Vec<u8> = Vec::new();
        // File position of buf[0].
        let mut offset: u64 = 0;
        let mut state = ChunkState::default();
        let mut eof = false;
        let mut want = chunk;
        let mut skipping_copy_data = false;
        let mut first = true;

        loop {
            if self.cancel.is_cancelled() {
                return Ok(Finish::Cancelled);
            }
            if !eof {
                let read = (&mut file)
                    .take(want as u64)
                    .read_to_end(&mut buf)
                    .await
                    .map_err(|e| Error::Io(format!("cannot read {}: {e}", path.display())))?;
                eof = read < want;
            }
            if first {
                if buf.len() < UTF8_BOM.len() && !eof {
                    continue; // too little to tell whether the file starts with a byte order mark
                }
                first = false;
                if buf.starts_with(UTF8_BOM) {
                    buf.drain(..UTF8_BOM.len());
                    offset += UTF8_BOM.len() as u64;
                } else if buf.starts_with(&[0xFF, 0xFE]) || buf.starts_with(&[0xFE, 0xFF]) {
                    return Err(Error::InvalidInput(format!(
                        "{} is UTF-16 text. Save it as UTF-8 and import it again.",
                        path.display()
                    )));
                }
            }

            if skipping_copy_data {
                match copy_data_end(&buf, eof) {
                    CopyEnd::At(end) => {
                        buf.drain(..end);
                        offset += end as u64;
                    }
                    CopyEnd::Pending(keep_from) => {
                        buf.drain(..keep_from);
                        offset += keep_from as u64;
                        self.bytes = offset;
                        if eof {
                            self.bytes += buf.len() as u64;
                            break; // the file ends inside the data block
                        }
                        continue;
                    }
                }
            }

            // Only whole characters can be split; the bytes of one cut by the end of the piece
            // wait for the next piece.
            let text = match std::str::from_utf8(&buf) {
                Ok(text) => text,
                Err(e) if e.error_len().is_none() && !eof => {
                    std::str::from_utf8(&buf[..e.valid_up_to()]).unwrap_or_default()
                }
                Err(e) => {
                    return Err(Error::InvalidInput(format!(
                        "{} isn't valid UTF-8 (first bad byte at position {}). Save the file as \
                         UTF-8; for a mysqldump with binary columns, dump again with --hex-blob.",
                        path.display(),
                        offset + e.valid_up_to() as u64
                    )));
                }
            };
            let last = eof && text.len() == buf.len();
            let (spans, consumed) = split_chunk(text, self.dialect, &mut state, last);

            let mut copy_data_from: Option<usize> = None;
            for span in &spans {
                match self.statement(span).await {
                    Flow::Next => {}
                    Flow::SkipCopyData => {
                        copy_data_from = Some(
                            text[span.end..]
                                .find('\n')
                                .map_or(text.len(), |p| span.end + p + 1),
                        );
                        self.bytes = offset + span.end as u64;
                        break;
                    }
                    Flow::Stop(message) => return Ok(Finish::Stopped(message)),
                    Flow::Cancelled => return Ok(Finish::Cancelled),
                }
                self.bytes = offset + span.end as u64;
                self.reporter.tick(self.ran, self.bytes);
            }

            let used = copy_data_from.unwrap_or(consumed);
            skipping_copy_data = copy_data_from.is_some();
            buf.drain(..used);
            offset += used as u64;
            if last && !skipping_copy_data {
                self.bytes = offset;
                break;
            }
            // A statement bigger than the piece: read as much again, so the work stays linear.
            want = if used == 0 {
                want.max(buf.len())
            } else {
                chunk
            };
        }
        Ok(Finish::Completed)
    }

    /// Runs one statement of the file.
    async fn statement(&mut self, span: &StatementSpan) -> Flow {
        let mut sql = span.sql.as_str();
        let mut kind = span.kind;

        if self.dialect == Dialect::Postgres && sql.starts_with('\\') {
            let (commands, rest) = psql_meta(sql);
            for command in commands {
                match meta_action(command) {
                    Meta::Ignore => {}
                    Meta::Unsupported(message) => {
                        if let Some(stop) = self.skippable_failure(message) {
                            return stop;
                        }
                    }
                    Meta::Fatal(message) => return self.fatal(message),
                }
            }
            sql = rest;
            if sql.is_empty() {
                return Flow::Next;
            }
            kind = statement_kind(sql, self.dialect);
        }

        if self.read_only && kind != StatementKind::Read {
            return self.fatal(READ_ONLY.to_owned());
        }
        if self.dialect == Dialect::Postgres && is_copy_from_stdin(sql) {
            return match self.skippable_failure(COPY_STDIN.to_owned()) {
                Some(stop) => stop,
                None => Flow::SkipCopyData,
            };
        }

        self.attempted += 1;
        let number = self.attempted;
        match self.execute(sql).await {
            Ok(()) => {
                self.ran += 1;
                if let Some(open) = transaction_effect(sql) {
                    self.open_transaction = open;
                }
                Flow::Next
            }
            Err(Error::Cancelled) if self.cancel.is_cancelled() => Flow::Cancelled,
            Err(e) => {
                let message = e.to_string();
                // A lost connection fails every statement after it as well: not worth going on.
                let hopeless = matches!(e, Error::Connection(_) | Error::Ssh(_));
                if self.stop_on_error || hopeless {
                    Flow::Stop(format!("Statement {number} failed: {message}"))
                } else {
                    self.note_failure(number, &message);
                    Flow::Next
                }
            }
        }
    }

    /// A failure of a statement that was not even sent: it ends the job unless failures are
    /// being skipped.
    fn skippable_failure(&mut self, message: String) -> Option<Flow> {
        self.attempted += 1;
        let number = self.attempted;
        if self.stop_on_error {
            Some(Flow::Stop(format!("Statement {number} failed: {message}")))
        } else {
            self.note_failure(number, &message);
            None
        }
    }

    /// A failure that ends the job whatever `stop_on_error` says.
    fn fatal(&mut self, message: String) -> Flow {
        self.attempted += 1;
        Flow::Stop(format!("Statement {} failed: {message}", self.attempted))
    }

    fn note_failure(&mut self, number: u64, message: &str) {
        self.failed += 1;
        self.first_failure
            .get_or_insert_with(|| format!("statement {number}: {message}"));
    }

    /// Sends one statement and reads its result to the end.
    async fn execute(&self, sql: &str) -> Result<()> {
        let mut cursor = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(Error::Cancelled),
            opened = self.driver.query(self.database.as_deref(), sql) => opened?,
        };
        let handle = cursor.cancel_handle();
        loop {
            let event = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => {
                    let _ = tokio::time::timeout(CANCEL_WAIT, handle.cancel()).await;
                    return Err(Error::Cancelled);
                }
                event = cursor.next(BATCH) => event?,
            };
            if event.is_none() {
                return Ok(());
            }
        }
    }

    /// Ends the job: closes a transaction the file left open and builds the final progress.
    async fn conclude(self, finish: Finish) -> JobProgress {
        let rolled_back = if self.open_transaction {
            // The transaction would otherwise stay open on the connection the next query reuses.
            let rollback = tokio::time::timeout(CANCEL_WAIT, async {
                let mut cursor = self
                    .driver
                    .query(self.database.as_deref(), "ROLLBACK")
                    .await?;
                while cursor.next(BATCH).await?.is_some() {}
                Ok::<(), Error>(())
            })
            .await;
            matches!(rollback, Ok(Ok(())))
        } else {
            false
        };
        // Before the job reports that it is done, so whatever it changed is already settled
        // (the session forgets what it knew about the tables) when the UI looks again.
        if let Some(script) = &self.script {
            script.finish().await;
        }
        let note = " The transaction the file had opened was rolled back.";

        let mut done = JobProgress {
            done: true,
            ..progress_of(self.ran, self.bytes)
        };
        match finish {
            Finish::Completed => {
                let mut warnings = Vec::new();
                if self.failed > 0 {
                    warnings.push(format!(
                        "{} of {} statements failed; first: {}",
                        self.failed,
                        self.attempted,
                        self.first_failure.as_deref().unwrap_or("unknown")
                    ));
                }
                if rolled_back {
                    warnings.push(
                        "The file ended inside a transaction (no COMMIT), so it was rolled back."
                            .to_owned(),
                    );
                }
                if self.attempted == 0 {
                    warnings.push("The file contains no SQL statements.".to_owned());
                }
                done.warning = (!warnings.is_empty()).then(|| warnings.join(" "));
            }
            Finish::Cancelled => done.cancelled = true,
            Finish::Stopped(message) | Finish::Failed(message) => {
                let mut message = message;
                if rolled_back {
                    message.push_str(note);
                }
                done.error = Some(message);
            }
        }
        self.reporter.finish(&done);
        done
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL dump specifics
// ---------------------------------------------------------------------------

static COPY_FROM_STDIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)^\s*COPY\b.*?\bFROM\s+STDIN\b").expect("valid pattern"));

/// `COPY t (a, b) FROM stdin`: the rows follow the statement in the file.
fn is_copy_from_stdin(sql: &str) -> bool {
    sql.get(..4).is_some_and(|w| w.eq_ignore_ascii_case("copy")) && COPY_FROM_STDIN.is_match(sql)
}

/// Where the data lines of a `COPY ... FROM stdin` block end.
enum CopyEnd {
    /// The block's `\.` line ends here (offset just after it).
    At(usize),
    /// Not yet seen; everything before this offset can be dropped.
    Pending(usize),
}

fn copy_data_end(buf: &[u8], eof: bool) -> CopyEnd {
    let mut start = 0;
    while start < buf.len() {
        let newline = buf[start..].iter().position(|&b| b == b'\n');
        let (line, next) = match newline {
            Some(p) => (&buf[start..start + p], start + p + 1),
            None => (&buf[start..], buf.len()),
        };
        if line.strip_suffix(b"\r").unwrap_or(line) == b"\\." && (newline.is_some() || eof) {
            return CopyEnd::At(next);
        }
        if newline.is_none() {
            return CopyEnd::Pending(start); // a line that may continue in the next piece
        }
        start = next;
    }
    CopyEnd::Pending(buf.len())
}

enum Meta {
    /// Nothing to do: pg_dump's own bookkeeping.
    Ignore,
    /// Cannot be run here; the job may go on without it.
    Unsupported(String),
    /// Changes where the rest of the file would run: going on would be wrong.
    Fatal(String),
}

/// The psql meta-command lines at the start of `sql` (without the backslash) and what follows.
fn psql_meta(sql: &str) -> (Vec<&str>, &str) {
    let mut commands = Vec::new();
    let mut rest = sql.trim_start();
    while let Some(line) = rest.strip_prefix('\\') {
        let (line, after) = line.split_once('\n').unwrap_or((line, ""));
        commands.push(line.trim_end());
        rest = after.trim_start();
    }
    (commands, rest.trim_end())
}

fn meta_action(command: &str) -> Meta {
    let name = command.split_whitespace().next().unwrap_or_default();
    match name {
        // pg_dump 17.6+ brackets a dump in \restrict KEY ... \unrestrict KEY.
        "restrict" | "unrestrict" | "set" | "unset" | "echo" | "qecho" | "encoding" | "timing" => {
            Meta::Ignore
        }
        "connect" | "c" => Meta::Fatal(format!(
            "\\{name} switches database, which a plain import can't follow. Open the target \
             database and import the file there."
        )),
        _ => Meta::Unsupported(format!("The psql command \\{name} can't be run here.")),
    }
}

/// `Some(true)` when the statement opens a transaction, `Some(false)` when it closes one.
fn transaction_effect(sql: &str) -> Option<bool> {
    let mut words = sql
        .split(|c: char| c.is_whitespace() || c == ';')
        .filter(|w| !w.is_empty());
    let first = words.next()?.to_ascii_uppercase();
    match first.as_str() {
        "BEGIN" => Some(true),
        "START" => words
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("transaction"))
            .then_some(true),
        "COMMIT" | "END" => Some(false),
        "ROLLBACK" => {
            let to_savepoint = words.next().is_some_and(|w| w.eq_ignore_ascii_case("to"));
            (!to_savepoint).then_some(false)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::sqlite;
    use crate::model::{ConnectParams, ConnectionConfig, EnvTag};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[test]
    fn the_spec_has_the_json_shape_the_ui_sends() {
        let spec: SqlImport =
            serde_json::from_str(r#"{"path":"/tmp/a.sql","database":null,"stopOnError":false}"#)
                .unwrap();
        assert_eq!(
            spec,
            SqlImport {
                path: "/tmp/a.sql".into(),
                database: None,
                stop_on_error: false,
            }
        );
        let spec: SqlImport =
            serde_json::from_str(r#"{"path":"/tmp/a.sql","database":"shop","stopOnError":true}"#)
                .unwrap();
        assert_eq!(spec.database.as_deref(), Some("shop"));
        assert!(spec.stop_on_error);
    }

    #[test]
    fn copy_blocks_end_at_their_terminator_line() {
        let data = b"1\tone\n2\ttwo\n\\.\nSELECT 1;\n";
        assert!(matches!(copy_data_end(data, false), CopyEnd::At(15)));
        // CRLF files.
        assert!(matches!(
            copy_data_end(b"1\r\n\\.\r\nx", false),
            CopyEnd::At(7)
        ));
        // A line that merely starts like the terminator is data.
        assert!(matches!(
            copy_data_end(b"\\.x\n\\N\n", false),
            CopyEnd::Pending(7)
        ));
        // A terminator cut off by the end of the piece waits for more, unless the file ended.
        assert!(matches!(
            copy_data_end(b"a\n\\.", false),
            CopyEnd::Pending(2)
        ));
        assert!(matches!(copy_data_end(b"a\n\\.", true), CopyEnd::At(4)));
        // Only the unfinished last line is kept.
        assert!(matches!(
            copy_data_end(b"abc\ndef", false),
            CopyEnd::Pending(4)
        ));
        assert!(matches!(copy_data_end(b"", false), CopyEnd::Pending(0)));
    }

    #[test]
    fn recognises_copy_from_stdin() {
        assert!(is_copy_from_stdin("COPY public.t (a, b) FROM stdin"));
        assert!(is_copy_from_stdin("copy t from STDIN with (format csv)"));
        assert!(is_copy_from_stdin("COPY t (\n  a\n) FROM\n stdin"));
        assert!(!is_copy_from_stdin("COPY t FROM '/tmp/data.csv'"));
        assert!(!is_copy_from_stdin("COPY t TO stdout"));
        assert!(!is_copy_from_stdin("INSERT INTO copy_stdin VALUES (1)"));
        assert!(!is_copy_from_stdin("SELECT 'COPY x FROM stdin'"));
    }

    #[test]
    fn psql_meta_lines_are_split_off() {
        assert_eq!(
            psql_meta("\\restrict abc123\n\nSET statement_timeout = 0"),
            (vec!["restrict abc123"], "SET statement_timeout = 0")
        );
        assert_eq!(
            psql_meta("\\connect shop\r\n\\set x 1\nCREATE TABLE t (a int)"),
            (vec!["connect shop", "set x 1"], "CREATE TABLE t (a int)")
        );
        assert_eq!(
            psql_meta("\\unrestrict abc123"),
            (vec!["unrestrict abc123"], "")
        );
        assert!(matches!(meta_action("restrict k"), Meta::Ignore));
        assert!(matches!(meta_action("connect shop"), Meta::Fatal(_)));
        assert!(matches!(meta_action("c shop"), Meta::Fatal(_)));
        assert!(matches!(
            meta_action("copy t from 'f'"),
            Meta::Unsupported(_)
        ));
    }

    #[test]
    fn follows_transactions() {
        assert_eq!(transaction_effect("BEGIN TRANSACTION;"), Some(true));
        assert_eq!(transaction_effect("begin"), Some(true));
        assert_eq!(transaction_effect("START TRANSACTION"), Some(true));
        assert_eq!(transaction_effect("START SLAVE"), None);
        assert_eq!(transaction_effect("COMMIT"), Some(false));
        assert_eq!(transaction_effect("END TRANSACTION"), Some(false));
        assert_eq!(transaction_effect("ROLLBACK"), Some(false));
        assert_eq!(transaction_effect("ROLLBACK TO SAVEPOINT a"), None);
        assert_eq!(transaction_effect("SELECT 1"), None);
    }

    // --- end to end against an in-memory SQLite -------------------------------

    fn config() -> ConnectionConfig {
        ConnectionConfig {
            id: String::new(),
            name: "test".into(),
            engine: Engine::Sqlite,
            host: String::new(),
            port: 0,
            user: String::new(),
            database: String::new(),
            file_path: Some(":memory:".into()),
            url: None,
            ssl: Default::default(),
            ssh: None,
            env: EnvTag::Local,
            color: None,
            read_only: false,
            options: BTreeMap::new(),
        }
    }

    async fn driver() -> Arc<dyn Driver> {
        sqlite::connect(&ConnectParams {
            config: config(),
            password: None,
            host: String::new(),
            port: 0,
        })
        .await
        .unwrap()
    }

    fn write_file(contents: &[u8]) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "tablesail-sqlimport-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("dump.sql");
        std::fs::write(&file, contents).unwrap();
        (dir, file)
    }

    struct Run {
        progress: JobProgress,
        reports: Vec<JobProgress>,
    }

    async fn import(
        d: &Arc<dyn Driver>,
        contents: &[u8],
        dialect: Dialect,
        stop_on_error: bool,
        read_only: bool,
        chunk: usize,
    ) -> Run {
        let (dir, file) = write_file(contents);
        let reports = Arc::new(Mutex::new(Vec::new()));
        let sink = reports.clone();
        let progress: ProgressFn = Arc::new(move |p| sink.lock().unwrap().push(p.clone()));
        let result = run_chunked(
            d.clone(),
            dialect,
            SqlImport {
                path: file.to_string_lossy().into_owned(),
                database: None,
                stop_on_error,
            },
            read_only,
            progress,
            CancellationToken::new(),
            chunk,
            true,
        )
        .await;
        std::fs::remove_dir_all(dir).unwrap();
        let reports = reports.lock().unwrap().clone();
        Run {
            progress: result.unwrap(),
            reports,
        }
    }

    async fn count(d: &Arc<dyn Driver>, sql: &str) -> i64 {
        let mut cursor = d.query(None, sql).await.unwrap();
        let mut n = None;
        while let Some(event) = cursor.next(10).await.unwrap() {
            if let crate::driver::CursorEvent::Rows(rows) = event {
                n = match &rows[0][0] {
                    crate::value::Value::Int(i) => Some(*i),
                    other => panic!("{other:?}"),
                };
            }
        }
        n.unwrap()
    }

    const DUMP: &str = "-- a dump\n\
        CREATE TABLE t (id INTEGER PRIMARY KEY, s TEXT);\n\
        INSERT INTO t VALUES (1, 'héllo; wörld ✓');\n\
        /* block ; comment */ INSERT INTO t VALUES (2, 'it''s');\n\
        INSERT INTO t VALUES (3, \"x\");\n\
        CREATE TRIGGER trg AFTER INSERT ON t BEGIN UPDATE t SET s = s || '!' WHERE id = new.id; END;\n\
        INSERT INTO t VALUES (4, 'trigger');\n";

    #[tokio::test]
    async fn runs_every_statement_whatever_the_piece_size() {
        for chunk in [1, 2, 3, 5, 7, 16, 64, 1000, CHUNK] {
            let d = driver().await;
            let run = import(&d, DUMP.as_bytes(), Dialect::Sqlite, true, false, chunk).await;
            let p = &run.progress;
            assert_eq!(p.error, None, "chunk {chunk}");
            assert_eq!(p.warning, None, "chunk {chunk}");
            assert!(p.done && !p.cancelled, "chunk {chunk}");
            assert_eq!(p.rows, 6, "chunk {chunk}");
            assert_eq!(p.bytes, DUMP.len() as u64, "chunk {chunk}");
            assert_eq!(
                count(&d, "SELECT count(*) FROM t").await,
                4,
                "chunk {chunk}"
            );
            // The trigger exists and was not cut in two at its inner semicolon.
            assert_eq!(
                count(&d, "SELECT count(*) FROM t WHERE s LIKE '%!'").await,
                1,
                "chunk {chunk}"
            );
            assert_eq!(run.reports.last(), Some(&run.progress));
        }
    }

    #[tokio::test]
    async fn multibyte_text_cut_by_the_piece_boundary_is_whole_again() {
        let sql = "CREATE TABLE u (v TEXT);\nINSERT INTO u VALUES ('日本語🙂é');\n";
        for chunk in 1..=9 {
            let d = driver().await;
            let run = import(&d, sql.as_bytes(), Dialect::Sqlite, true, false, chunk).await;
            assert_eq!(
                (run.progress.rows, &run.progress.error),
                (2, &None),
                "chunk {chunk}"
            );
            assert_eq!(
                count(&d, "SELECT length(v) FROM u").await,
                5,
                "chunk {chunk}: characters, not bytes"
            );
        }
    }

    #[tokio::test]
    async fn stops_at_the_first_error_with_its_number() {
        let d = driver().await;
        let sql = "CREATE TABLE t (a INT);\nINSERT INTO t VALUES (1);\nINSERT INTO nope VALUES (2);\nINSERT INTO t VALUES (3);";
        let run = import(&d, sql.as_bytes(), Dialect::Sqlite, true, false, CHUNK).await;
        let p = run.progress;
        assert!(p.done && !p.cancelled);
        let error = p.error.expect("an error");
        assert!(
            error.starts_with("Statement 3 failed: ") && error.contains("no such table: nope"),
            "{error}"
        );
        assert_eq!(p.rows, 2);
        assert_eq!(count(&d, "SELECT count(*) FROM t").await, 1);
    }

    #[tokio::test]
    async fn carries_on_after_errors_and_counts_them() {
        let d = driver().await;
        let sql = "CREATE TABLE t (a INT);\nINSERT INTO nope VALUES (1);\nINSERT INTO t VALUES (2);\nBOGUS;\nINSERT INTO t VALUES (3);";
        let run = import(&d, sql.as_bytes(), Dialect::Sqlite, false, false, 9).await;
        let p = run.progress;
        assert_eq!(p.error, None);
        assert_eq!(p.rows, 3);
        let warning = p.warning.expect("a warning");
        assert!(
            warning.starts_with("2 of 5 statements failed; first: statement 2: "),
            "{warning}"
        );
        assert!(warning.contains("no such table: nope"), "{warning}");
        assert_eq!(count(&d, "SELECT count(*) FROM t").await, 2);
    }

    #[tokio::test]
    async fn a_read_only_connection_only_reads() {
        let d = driver().await;
        let run = import(
            &d,
            b"SELECT 1;\nSELECT 2;",
            Dialect::Sqlite,
            true,
            true,
            CHUNK,
        )
        .await;
        assert_eq!((run.progress.rows, run.progress.error), (2, None));

        let sql = "SELECT 1;\nCREATE TABLE t (a INT);\nSELECT 3;";
        for stop_on_error in [true, false] {
            let run = import(
                &d,
                sql.as_bytes(),
                Dialect::Sqlite,
                stop_on_error,
                true,
                CHUNK,
            )
            .await;
            // A write ends the job even when errors are being skipped, and never reaches the server.
            assert_eq!(
                run.progress.error.as_deref(),
                Some("Statement 2 failed: This connection is read-only.")
            );
            assert_eq!(run.progress.rows, 1);
            assert_eq!(
                count(&d, "SELECT count(*) FROM sqlite_master WHERE name = 't'").await,
                0
            );
        }
    }

    #[tokio::test]
    async fn bom_and_encodings() {
        let d = driver().await;
        let mut sql = b"\xEF\xBB\xBF".to_vec();
        sql.extend_from_slice(b"CREATE TABLE b (a INT);");
        let run = import(&d, &sql, Dialect::Sqlite, true, false, CHUNK).await;
        assert_eq!((run.progress.rows, run.progress.error), (1, None));
        assert_eq!(run.progress.bytes, sql.len() as u64);

        let run = import(
            &d,
            b"SELECT 1;\nSELECT 'caf\xE9';",
            Dialect::Sqlite,
            true,
            false,
            5,
        )
        .await;
        let error = run.progress.error.expect("an error");
        assert!(error.contains("isn't valid UTF-8"), "{error}");
        assert!(error.contains("position 21"), "{error}");
        assert!(error.contains("--hex-blob"), "{error}");

        let run = import(&d, b"\xFF\xFES\0E\0", Dialect::Sqlite, true, false, CHUNK).await;
        assert!(run.progress.error.expect("an error").contains("UTF-16"));
    }

    #[tokio::test]
    async fn an_empty_file_is_not_an_error_but_says_so() {
        let d = driver().await;
        for sql in ["", "  \n-- nothing here\n/* at all */\n"] {
            let run = import(&d, sql.as_bytes(), Dialect::Sqlite, true, false, CHUNK).await;
            assert_eq!((run.progress.rows, &run.progress.error), (0, &None));
            assert_eq!(
                run.progress.warning.as_deref(),
                Some("The file contains no SQL statements.")
            );
        }
    }

    #[tokio::test]
    async fn a_transaction_left_open_is_rolled_back() {
        let d = driver().await;
        let sql = "CREATE TABLE t (a INT);\nBEGIN TRANSACTION;\nINSERT INTO t VALUES (1);\nINSERT INTO nope VALUES (2);\nINSERT INTO t VALUES (3);\nCOMMIT;";
        let run = import(&d, sql.as_bytes(), Dialect::Sqlite, true, false, CHUNK).await;
        let error = run.progress.error.expect("an error");
        assert!(error.starts_with("Statement 4 failed: "), "{error}");
        assert!(
            error.ends_with("The transaction the file had opened was rolled back."),
            "{error}"
        );
        assert_eq!(count(&d, "SELECT count(*) FROM t").await, 0);

        // Ending without a COMMIT is rolled back too, with a warning.
        let run = import(
            &d,
            b"BEGIN;\nINSERT INTO t VALUES (7);",
            Dialect::Sqlite,
            true,
            false,
            CHUNK,
        )
        .await;
        assert_eq!(run.progress.error, None);
        assert!(
            run.progress
                .warning
                .expect("a warning")
                .contains("rolled back")
        );
        assert_eq!(count(&d, "SELECT count(*) FROM t").await, 0);

        // A properly closed one is committed and silent.
        let run = import(
            &d,
            b"BEGIN;\nINSERT INTO t VALUES (8);\nCOMMIT;",
            Dialect::Sqlite,
            true,
            false,
            CHUNK,
        )
        .await;
        assert_eq!((run.progress.error, run.progress.warning), (None, None));
        assert_eq!(count(&d, "SELECT count(*) FROM t").await, 1);
    }

    #[tokio::test]
    async fn state_set_by_the_file_stays_for_its_later_statements() {
        let d = driver().await;
        let sql = "CREATE TEMP TABLE scratch (a INT);\nINSERT INTO scratch VALUES (5);\nCREATE TABLE copied AS SELECT a FROM scratch;";
        let run = import(&d, sql.as_bytes(), Dialect::Sqlite, true, false, 11).await;
        assert_eq!((run.progress.rows, run.progress.error), (3, None));
        assert_eq!(count(&d, "SELECT a FROM copied").await, 5);
    }

    const PG_DUMP: &str = "--\n-- PostgreSQL database dump\n--\n\n\\restrict K3y\n\nPRAGMA user_version = 3;\nCREATE TABLE t (id INT, name TEXT);\nCOPY public.t (id, name) FROM stdin;\n1\tO'Brien\n2\ta;b\n3\t\"unbalanced\n\\.\n\nINSERT INTO t VALUES (9, 'after');\nSELECT 1;\n\\unrestrict K3y\n";

    #[tokio::test]
    async fn pg_dump_boilerplate_is_skipped_and_copy_blocks_fail_clearly() {
        let d = driver().await;
        // SQLite runs the statements; the dialect only decides how the file is read.
        let run = import(
            &d,
            PG_DUMP.as_bytes(),
            Dialect::Postgres,
            true,
            false,
            CHUNK,
        )
        .await;
        let error = run.progress.error.expect("an error");
        assert!(
            error.starts_with("Statement 3 failed: COPY ... FROM stdin"),
            "{error}"
        );
        assert!(error.contains("pg_dump --inserts"), "{error}");
        assert_eq!(run.progress.rows, 2); // PRAGMA + CREATE TABLE (\restrict is not a statement)

        // Skipping failures: the data lines are not executed, everything after the block is.
        for chunk in [1, 3, 8, 21, CHUNK] {
            let d = driver().await;
            let run = import(
                &d,
                PG_DUMP.as_bytes(),
                Dialect::Postgres,
                false,
                false,
                chunk,
            )
            .await;
            let p = run.progress;
            assert_eq!(p.error, None, "chunk {chunk}");
            let warning = p.warning.expect("a warning");
            assert!(
                warning.starts_with(
                    "1 of 5 statements failed; first: statement 3: COPY ... FROM stdin"
                ),
                "chunk {chunk}: {warning}"
            );
            assert_eq!(p.rows, 4, "chunk {chunk}");
            assert_eq!(p.bytes, PG_DUMP.len() as u64, "chunk {chunk}");
            assert_eq!(
                count(&d, "SELECT count(*) FROM t").await,
                1,
                "chunk {chunk}"
            );
            assert_eq!(count(&d, "SELECT id FROM t").await, 9, "chunk {chunk}");
        }
    }

    #[tokio::test]
    async fn psql_connect_ends_the_job_even_when_errors_are_skipped() {
        let d = driver().await;
        let sql = "SELECT 1;\n\\connect other\nCREATE TABLE t (a INT);";
        for stop_on_error in [true, false] {
            let run = import(
                &d,
                sql.as_bytes(),
                Dialect::Postgres,
                stop_on_error,
                false,
                CHUNK,
            )
            .await;
            let error = run.progress.error.expect("an error");
            assert!(
                error.starts_with("Statement 2 failed: \\connect switches database"),
                "{error}"
            );
            assert_eq!(run.progress.rows, 1);
        }
    }

    #[tokio::test]
    async fn mysql_delimiters_are_followed_across_pieces() {
        // Run as the MySQL dialect on SQLite: the splitter is what is being tested, with
        // statements both engines understand.
        let sql = "DELIMITER //\nCREATE TABLE d (a INT)//\nINSERT INTO d VALUES (1)//\nDELIMITER ;\nINSERT INTO d VALUES (2);\n";
        let mysql = Dialect::Mysql {
            backslash_escapes: true,
        };
        for chunk in [1, 4, 9, 30, CHUNK] {
            let d = driver().await;
            let run = import(&d, sql.as_bytes(), mysql, true, false, chunk).await;
            assert_eq!(
                (run.progress.rows, run.progress.error.clone()),
                (3, None),
                "chunk {chunk}"
            );
            assert_eq!(
                count(&d, "SELECT count(*) FROM d").await,
                2,
                "chunk {chunk}"
            );
        }
    }

    #[tokio::test]
    async fn cancelling_stops_the_job_without_an_error() {
        let d = driver().await;
        let mut sql = String::from("CREATE TABLE c (a INT);\n");
        for i in 0..60_000 {
            sql.push_str(&format!("INSERT INTO c VALUES ({i});\n"));
        }
        let (dir, file) = write_file(sql.as_bytes());
        let cancel = CancellationToken::new();
        // Progress is reported every ~100 ms; the first report cancels the job.
        let progress: ProgressFn = {
            let cancel = cancel.clone();
            Arc::new(move |p| {
                if !p.done {
                    cancel.cancel();
                }
            })
        };
        let result = run_chunked(
            d.clone(),
            Dialect::Sqlite,
            SqlImport {
                path: file.to_string_lossy().into_owned(),
                database: None,
                stop_on_error: true,
            },
            false,
            progress,
            cancel,
            CHUNK,
            true,
        )
        .await
        .unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert!(result.done && result.cancelled, "{result:?}");
        assert_eq!(result.error, None);
        assert!(result.rows >= 1 && result.rows < 60_001, "{result:?}");
        // Every statement that was counted ran, and nothing else did.
        assert_eq!(
            count(&d, "SELECT count(*) FROM c").await as u64,
            result.rows - 1
        );
    }

    #[tokio::test]
    async fn a_job_cancelled_before_it_starts_runs_nothing() {
        let d = driver().await;
        let (dir, file) = write_file(b"CREATE TABLE never (a INT);");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = run_chunked(
            d.clone(),
            Dialect::Sqlite,
            SqlImport {
                path: file.to_string_lossy().into_owned(),
                database: None,
                stop_on_error: true,
            },
            false,
            Arc::new(|_| {}),
            cancel,
            CHUNK,
            true,
        )
        .await
        .unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert!(result.done && result.cancelled);
        assert_eq!(result.rows, 0);
        assert_eq!(
            count(
                &d,
                "SELECT count(*) FROM sqlite_master WHERE name = 'never'"
            )
            .await,
            0
        );
    }

    #[tokio::test]
    async fn a_copy_block_that_runs_to_the_end_of_the_file_is_skipped() {
        for chunk in [2, 6, CHUNK] {
            let d = driver().await;
            let sql = "CREATE TABLE t (a INT);\nCOPY t FROM stdin;\n1\n2\n";
            let run = import(&d, sql.as_bytes(), Dialect::Postgres, false, false, chunk).await;
            let p = run.progress;
            assert_eq!((p.rows, &p.error), (1, &None), "chunk {chunk}");
            assert!(
                p.warning
                    .expect("a warning")
                    .starts_with("1 of 2 statements failed")
            );
            assert_eq!(p.bytes, sql.len() as u64, "chunk {chunk}");
        }
    }

    #[tokio::test]
    async fn preflight_problems_are_errors() {
        let d = driver().await;
        let progress: ProgressFn = Arc::new(|_| {});
        let run = |path: &str| {
            run_sql_import(
                d.clone(),
                SqlImport {
                    path: path.to_owned(),
                    database: None,
                    stop_on_error: true,
                },
                false,
                progress.clone(),
                CancellationToken::new(),
            )
        };
        assert!(matches!(run("").await, Err(Error::InvalidInput(_))));
        match run("/definitely/not/here.sql").await {
            Err(Error::Io(m)) => assert!(m.contains("here.sql"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            run(&std::env::temp_dir().to_string_lossy()).await,
            Err(Error::InvalidInput(_))
        ));
    }
}
