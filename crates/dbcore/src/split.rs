//! Splits a script into statements, respecting strings, quoted identifiers,
//! comments, dollar quoting (Postgres), DELIMITER (MySQL) and BEGIN…END bodies.

use crate::dialect::Dialect;
use crate::model::StatementSpan;

/// Splits `sql` into statements. Empty statements (only whitespace/comments) are dropped.
pub fn split_statements(sql: &str, dialect: Dialect) -> Vec<StatementSpan> {
    todo!("A1: split {} bytes for {dialect:?}", sql.len())
}

/// The statement containing (or nearest before) byte offset `cursor`.
pub fn statement_at(sql: &str, dialect: Dialect, cursor: usize) -> Option<StatementSpan> {
    let spans = split_statements(sql, dialect);
    spans
        .iter()
        .find(|s| cursor >= s.start && cursor <= s.end)
        .or_else(|| spans.iter().rev().find(|s| s.end <= cursor))
        .or(spans.first())
        .cloned()
}
