//! Process-wide log of the statements TableSoft sends to servers, for the
//! console pane. A bounded ring buffer: the newest [`CAPACITY`] entries are kept,
//! older ones fall off the front. Sequence numbers only ever grow, so a poller
//! can ask for "everything after the last entry I saw" even across a [`clear`].

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sqlx::AssertSqlSafe;

/// Entries kept in memory.
pub const CAPACITY: usize = 2000;

/// A single statement longer than this (e.g. a big import batch) is cut when stored.
const MAX_SQL_CHARS: usize = 20_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// 1-based, strictly increasing for the life of the process.
    pub seq: u64,
    /// Unix time in milliseconds when the statement was sent.
    pub at_ms: i64,
    pub sql: String,
}

struct Log {
    next: u64,
    cap: usize,
    entries: VecDeque<LogEntry>,
}

impl Log {
    const fn new(cap: usize) -> Self {
        Log {
            next: 1,
            cap,
            entries: VecDeque::new(),
        }
    }

    fn push(&mut self, at_ms: i64, sql: &str) {
        let entry = LogEntry {
            seq: self.next,
            at_ms,
            sql: clip(sql),
        };
        self.next += 1;
        if self.entries.len() >= self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    fn since(&self, seq: u64) -> Vec<LogEntry> {
        // Sequence numbers are contiguous, so the first match is found by arithmetic.
        let skip = match self.entries.front() {
            Some(first) if seq >= first.seq => (seq - first.seq + 1) as usize,
            _ => 0,
        };
        self.entries.iter().skip(skip).cloned().collect()
    }
}

fn clip(sql: &str) -> String {
    match sql.char_indices().nth(MAX_SQL_CHARS) {
        None => sql.to_owned(),
        Some((end, _)) => format!(
            "{}\n-- ... {} more characters not shown",
            &sql[..end],
            sql[end..].chars().count()
        ),
    }
}

static LOG: Mutex<Log> = Mutex::new(Log::new(CAPACITY));

fn log() -> MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Appends a statement to the log.
pub fn record(sql: &str) {
    log().push(now_ms(), sql);
}

/// Entries with a sequence number greater than `seq` (`0` returns everything kept).
pub fn since(seq: u64) -> Vec<LogEntry> {
    log().since(seq)
}

/// Forgets every entry. Sequence numbers keep counting.
pub fn clear() {
    log().entries.clear();
}

/// Records `sql` and wraps it for `sqlx::raw_sql`, so a call site only changes from
/// `AssertSqlSafe(sql)` to `logged(sql)`. The SQL is built by the drivers from
/// quoted identifiers and escaped literals, which is what `AssertSqlSafe` asserts.
pub fn logged(sql: impl Into<String>) -> AssertSqlSafe<String> {
    let sql = sql.into();
    record(&sql);
    AssertSqlSafe(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_entries_and_numbers_them() {
        let mut log = Log::new(3);
        for i in 1..=5 {
            log.push(i, &format!("SELECT {i}"));
        }
        let all = log.since(0);
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![3, 4, 5]);
        assert_eq!(all[0].sql, "SELECT 3");
        assert_eq!(all[2].at_ms, 5);
    }

    #[test]
    fn since_returns_only_newer_entries() {
        let mut log = Log::new(10);
        for i in 1..=4 {
            log.push(0, &format!("q{i}"));
        }
        assert_eq!(
            log.since(2).iter().map(|e| e.seq).collect::<Vec<_>>(),
            [3, 4]
        );
        assert!(log.since(4).is_empty());
        assert!(log.since(99).is_empty());
        assert_eq!(log.since(0).len(), 4);
    }

    #[test]
    fn since_after_eviction_starts_at_the_oldest_kept() {
        let mut log = Log::new(2);
        for i in 1..=6 {
            log.push(0, &format!("q{i}"));
        }
        // The poller last saw seq 1, which has already fallen off.
        assert_eq!(
            log.since(1).iter().map(|e| e.seq).collect::<Vec<_>>(),
            [5, 6]
        );
        assert_eq!(log.since(5).iter().map(|e| e.seq).collect::<Vec<_>>(), [6]);
    }

    #[test]
    fn numbering_survives_clearing() {
        let mut log = Log::new(5);
        log.push(0, "a");
        log.push(0, "b");
        log.entries.clear();
        log.push(0, "c");
        let all = log.since(0);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].seq, 3);
        assert_eq!(log.since(2).len(), 1);
    }

    #[test]
    fn long_statements_are_clipped_on_a_char_boundary() {
        let sql = "é".repeat(MAX_SQL_CHARS + 10);
        let clipped = clip(&sql);
        assert!(clipped.starts_with(&"é".repeat(MAX_SQL_CHARS)));
        assert!(clipped.ends_with("10 more characters not shown"));
        assert_eq!(clip("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn global_helpers_record_and_clear() {
        let marker = "SELECT 'querylog-global-test'";
        let wrapped = logged(marker);
        assert_eq!(wrapped.0, marker);
        assert!(since(0).iter().any(|e| e.sql == marker));
        clear();
        assert!(!since(0).iter().any(|e| e.sql == marker));
    }
}
