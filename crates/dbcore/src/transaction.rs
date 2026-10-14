//! Manual transactions. A transaction lives on a pinned driver (see `Driver::pinned`): one
//! dedicated connection that BEGIN, the statements and COMMIT/ROLLBACK all share, instead of
//! the pool, where each statement may land on a different connection.
//!
//! A script that controls transactions itself ("BEGIN; UPDATE …; COMMIT;" typed in the editor)
//! gets such a connection for its whole run (`ScriptConnection`), so it is atomic too.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::Engine;
use crate::dialect::Dialect;
use crate::driver::Driver;
use crate::error::{Error, Result};
use crate::model::StatementSpan;
use crate::split::{TxControl, transaction_control};

/// Longest the final ROLLBACK, and closing the connection, wait for a statement that is still
/// running on it.
const END_WAIT: Duration = Duration::from_secs(10);

/// SQLSTATE of "current transaction is aborted, commands ignored until end of transaction block".
const ABORTED: &str = "25P02";

/// A pinned driver and the transaction begun on it (a `ScriptConnection` leaves the BEGIN to its script).
pub struct Transaction {
    driver: Arc<dyn Driver>,
    /// The driver it was opened from: its caches never see the statements run here.
    parent: Arc<dyn Driver>,
    database: Option<String>,
}

impl Transaction {
    /// Opens a dedicated connection (in `database` when given) and begins a transaction on it.
    /// Every statement goes through the driver, so BEGIN shows up in the query log.
    pub async fn begin(driver: &Arc<dyn Driver>, database: Option<&str>) -> Result<Transaction> {
        let tx = Transaction::scoped(driver, database).await?;
        if let Err(e) = tx.run(begin_sql(driver.engine())).await {
            tx.close().await;
            return Err(e);
        }
        Ok(tx)
    }

    /// A dedicated connection without a transaction.
    async fn scoped(driver: &Arc<dyn Driver>, database: Option<&str>) -> Result<Transaction> {
        let database = database.filter(|d| !d.is_empty()).map(str::to_owned);
        let pinned = driver.pinned(database.as_deref()).await?;
        Ok(Transaction {
            driver: pinned,
            parent: driver.clone(),
            database,
        })
    }

    /// The driver whose statements run inside the transaction.
    pub fn driver(&self) -> &Arc<dyn Driver> {
        &self.driver
    }

    pub fn database(&self) -> Option<&str> {
        self.database.as_deref()
    }

    /// Commits. The connection stays open and the caller [`close`](Transaction::close)s it once
    /// this succeeded, so a COMMIT the server refuses (SQLite's "database is locked", a deferred
    /// constraint) can be retried or rolled back, see [`survives`](Transaction::survives). A
    /// Postgres transaction that failed earlier is refused here, because the server would
    /// answer COMMIT by quietly rolling back. The refusal is a `Database` error (SQLSTATE
    /// 25P02, like the server's own answer to any other statement): the kind `InvalidInput`
    /// is for a transaction that does not exist.
    pub async fn commit(&self) -> Result<()> {
        if self.driver.transaction_aborted().await? {
            return Err(Error::Database {
                message: "The transaction hit an error and can only be rolled back.".to_owned(),
                code: Some(ABORTED.to_owned()),
                statement: None,
            });
        }
        self.run("COMMIT").await.map_err(|e| match e {
            // Postgres has rolled the transaction back by the time it reports the failure.
            Error::Database {
                message,
                code,
                statement,
            } if self.parent.engine() == Engine::Postgres => Error::Database {
                message: format!("{} The transaction was rolled back.", message.trim_end()),
                code,
                statement,
            },
            other => other,
        })
    }

    /// Whether the transaction is still there to retry or roll back after `commit` failed with
    /// `error`. Postgres ends it when its COMMIT fails (the only thing it leaves open is our own
    /// refusal of an aborted one), so a statement sent afterwards would silently run on its own.
    pub fn survives(&self, error: &Error) -> bool {
        match (self.parent.engine(), error) {
            (Engine::Postgres, Error::Database { code, .. }) => code.as_deref() == Some(ABORTED),
            _ => true,
        }
    }

    /// Rolls back, then closes the connection. Never fails: a connection that is already gone
    /// has no transaction left to roll back.
    pub async fn rollback(&self) {
        self.end(true).await;
    }

    /// Closes the connection. Whatever transaction is still open on it is rolled back by the
    /// server.
    pub async fn close(&self) {
        self.end(false).await;
    }

    /// Closes the connection after an explicit ROLLBACK (`rollback`) or without (`close`).
    async fn end(&self, rollback: bool) {
        if rollback {
            let _ = tokio::time::timeout(END_WAIT, self.run("ROLLBACK")).await;
        }
        let _ = tokio::time::timeout(END_WAIT, self.driver.close()).await;
        // Whatever the transaction did to the schema is now settled.
        self.parent.forget_structures();
    }

    async fn run(&self, sql: &str) -> Result<()> {
        let mut cursor = self.driver.query(self.database.as_deref(), sql).await?;
        while cursor.next(1000).await?.is_some() {}
        Ok(())
    }
}

/// The message for a transaction id that is not (or no longer) open.
pub const NO_TRANSACTION: &str = "That transaction is no longer open.";

/// A transaction in the [`Transactions`] registry.
pub struct OpenTransaction {
    pub id: String,
    pub session_id: String,
    tx: Transaction,
}

impl OpenTransaction {
    /// The driver whose statements run inside the transaction.
    pub fn driver(&self) -> &Arc<dyn Driver> {
        self.tx.driver()
    }

    pub fn database(&self) -> Option<&str> {
        self.tx.database()
    }
}

/// The open manual transactions of the app, by id. Each belongs to one session, and ends
/// with its commit or rollback, or with the session.
#[derive(Default)]
pub struct Transactions {
    open: Mutex<HashMap<String, Arc<OpenTransaction>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Transactions {
    /// Begins a transaction on a dedicated connection of `driver` and returns its id.
    pub async fn begin(
        &self,
        session_id: &str,
        driver: &Arc<dyn Driver>,
        database: Option<&str>,
    ) -> Result<String> {
        let tx = Transaction::begin(driver, database).await?;
        let id = uuid::Uuid::new_v4().to_string();
        lock(&self.open).insert(
            id.clone(),
            Arc::new(OpenTransaction {
                id: id.clone(),
                session_id: session_id.to_owned(),
                tx,
            }),
        );
        Ok(id)
    }

    /// The open transaction `id`, which must belong to `session_id` when that is given.
    pub fn get(&self, id: &str, session_id: Option<&str>) -> Result<Arc<OpenTransaction>> {
        lock(&self.open)
            .get(id)
            .filter(|tx| session_id.is_none_or(|s| tx.session_id == s))
            .cloned()
            .ok_or_else(|| Error::InvalidInput(NO_TRANSACTION.to_owned()))
    }

    pub fn ids_of(&self, session_id: &str) -> Vec<String> {
        lock(&self.open)
            .values()
            .filter(|tx| tx.session_id == session_id)
            .map(|tx| tx.id.clone())
            .collect()
    }

    /// Commits and closes. A COMMIT the server refuses leaves the transaction open when the
    /// engine does (see [`Transaction::survives`]); roll it back to get rid of it.
    pub async fn commit(&self, id: &str) -> Result<()> {
        // Taken out first, so a second commit or rollback of the same id finds nothing.
        let tx = self.take(id)?;
        match tx.tx.commit().await {
            Ok(()) => {
                tx.tx.close().await;
                Ok(())
            }
            Err(e) if tx.tx.survives(&e) => {
                lock(&self.open).insert(tx.id.clone(), tx);
                Err(e)
            }
            Err(e) => {
                tx.tx.close().await;
                Err(e)
            }
        }
    }

    pub async fn rollback(&self, id: &str) -> Result<()> {
        let tx = self.take(id)?;
        tx.tx.rollback().await;
        Ok(())
    }

    /// Rolls back every transaction of the session (its connection is closing).
    pub async fn rollback_session(&self, session_id: &str) {
        for id in self.ids_of(session_id) {
            let _ = self.rollback(&id).await;
        }
    }

    /// Rolls back every transaction (the app is quitting).
    pub async fn rollback_all(&self) {
        let ids: Vec<String> = lock(&self.open).keys().cloned().collect();
        for id in ids {
            let _ = self.rollback(&id).await;
        }
    }

    fn take(&self, id: &str) -> Result<Arc<OpenTransaction>> {
        lock(&self.open)
            .remove(id)
            .ok_or_else(|| Error::InvalidInput(NO_TRANSACTION.to_owned()))
    }
}

/// A dedicated connection for one script that controls transactions itself ("BEGIN; UPDATE …;
/// COMMIT;" typed in the editor), so that the whole script lands on the same connection.
pub struct ScriptConnection {
    tx: Transaction,
    leaves_open: bool,
}

impl ScriptConnection {
    /// A connection for the script, or `None` when it has no transaction control and can use the pool.
    pub async fn open(
        driver: &Arc<dyn Driver>,
        database: Option<&str>,
        statements: &[StatementSpan],
    ) -> Result<Option<ScriptConnection>> {
        let dialect = driver.dialect();
        if !needs_own_connection(statements, dialect) {
            return Ok(None);
        }
        Ok(Some(ScriptConnection {
            tx: Transaction::scoped(driver, database).await?,
            leaves_open: leaves_transaction_open(statements, dialect),
        }))
    }

    /// A connection for a script that is run from a file (see `sqlimport`): whatever it sets
    /// (`SET`, `USE`, `BEGIN`, temporary tables) holds for every statement after it, however
    /// busy the session's other connections are. Closing it ends the transaction the file
    /// left open, if any.
    pub async fn dedicated(
        driver: &Arc<dyn Driver>,
        database: Option<&str>,
    ) -> Result<ScriptConnection> {
        Ok(ScriptConnection {
            tx: Transaction::scoped(driver, database).await?,
            leaves_open: false,
        })
    }

    pub fn driver(&self) -> &Arc<dyn Driver> {
        self.tx.driver()
    }

    /// Closes the connection once the script is over. A transaction it left open is rolled
    /// back (closing the connection would roll it back too; the ROLLBACK makes it visible).
    pub async fn finish(&self) {
        self.tx.end(self.leaves_open).await;
    }
}

fn begin_sql(engine: Engine) -> &'static str {
    match engine {
        Engine::Mysql | Engine::Mariadb => "START TRANSACTION",
        _ => "BEGIN",
    }
}

/// Whether the script has a statement that only works on one connection (BEGIN, COMMIT,
/// SAVEPOINT, `SET autocommit` …), so it must not be spread over the pool.
pub fn needs_own_connection(statements: &[StatementSpan], dialect: Dialect) -> bool {
    statements
        .iter()
        .any(|s| transaction_control(&s.sql, dialect).is_some())
}

/// Whether the script, read top to bottom, ends inside a transaction it began itself.
pub fn leaves_transaction_open(statements: &[StatementSpan], dialect: Dialect) -> bool {
    let mut open = false;
    for s in statements {
        match transaction_control(&s.sql, dialect) {
            Some(TxControl::Begin) => open = true,
            Some(TxControl::Commit | TxControl::Rollback) => open = false,
            _ => {}
        }
    }
    open
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::split_statements;

    const PG: Dialect = Dialect::Postgres;
    const LITE: Dialect = Dialect::Sqlite;
    const MY: Dialect = Dialect::Mysql {
        backslash_escapes: true,
    };

    fn spans(sql: &str, dialect: Dialect) -> Vec<StatementSpan> {
        split_statements(sql, dialect)
    }

    #[test]
    fn scripts_with_transaction_control_need_their_own_connection() {
        assert!(needs_own_connection(
            &spans("BEGIN; UPDATE t SET a = 1; COMMIT;", PG),
            PG
        ));
        assert!(needs_own_connection(
            &spans("SAVEPOINT s; SELECT 1", LITE),
            LITE
        ));
        assert!(needs_own_connection(
            &spans("SET autocommit = 0; SELECT 1", MY),
            MY
        ));
        assert!(!needs_own_connection(
            &spans("SELECT 1; UPDATE t SET a = 1", PG),
            PG
        ));
        assert!(!needs_own_connection(
            &spans("SET search_path = a; SELECT 1", PG),
            PG
        ));
        assert!(!needs_own_connection(&[], PG));
        // A mongo shell script has no transactions of its own.
        let mongo = Dialect::Mongo;
        assert!(!needs_own_connection(&spans("db.t.find()", mongo), mongo));
    }

    #[test]
    fn classifies_transaction_statements() {
        use TxControl::*;
        let cases: [(&str, Dialect, Option<TxControl>); 26] = [
            ("BEGIN", PG, Some(Begin)),
            ("begin transaction", LITE, Some(Begin)),
            ("BEGIN IMMEDIATE", LITE, Some(Begin)),
            ("START TRANSACTION READ ONLY", MY, Some(Begin)),
            ("start  transaction", PG, Some(Begin)),
            ("-- go\nCOMMIT", PG, Some(Commit)),
            ("END", LITE, Some(Commit)),
            ("COMMIT WORK", MY, Some(Commit)),
            ("ROLLBACK", PG, Some(Rollback)),
            ("rollback work", MY, Some(Rollback)),
            ("ABORT", PG, Some(Rollback)),
            ("ROLLBACK TO SAVEPOINT s", PG, Some(Savepoint)),
            ("ROLLBACK TO s", LITE, Some(Savepoint)),
            ("ROLLBACK TRANSACTION TO s", LITE, Some(Savepoint)),
            ("SAVEPOINT s", PG, Some(Savepoint)),
            ("RELEASE SAVEPOINT s", MY, Some(Savepoint)),
            ("SET autocommit = 0", MY, Some(Settings)),
            ("SET @@autocommit=0", MY, Some(Settings)),
            ("SET SESSION autocommit = 1", MY, Some(Settings)),
            (
                "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
                PG,
                Some(Settings),
            ),
            (
                "SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED",
                MY,
                Some(Settings),
            ),
            ("SET search_path = a", PG, None),
            ("SET @x = 1", MY, None),
            ("SELECT 'BEGIN'", PG, None),
            ("UPDATE begin SET a = 1", PG, None),
            ("START SLAVE", MY, None),
        ];
        for (sql, dialect, expected) in cases {
            assert_eq!(transaction_control(sql, dialect), expected, "{sql}");
        }
    }

    #[test]
    fn knows_whether_a_script_leaves_a_transaction_open() {
        let open = |sql: &str| leaves_transaction_open(&spans(sql, PG), PG);
        assert!(open("BEGIN; UPDATE t SET a = 1"));
        assert!(!open("BEGIN; UPDATE t SET a = 1; COMMIT"));
        assert!(!open("BEGIN; UPDATE t SET a = 1; ROLLBACK"));
        assert!(open("BEGIN; COMMIT; BEGIN; SELECT 1"));
        assert!(open("BEGIN; SAVEPOINT s; ROLLBACK TO s"));
        assert!(!open("SELECT 1"));
    }
}
