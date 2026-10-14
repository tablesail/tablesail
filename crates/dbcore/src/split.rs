//! Splits a script into statements, respecting strings, quoted identifiers,
//! comments, dollar quoting (Postgres), DELIMITER (MySQL) and BEGIN…END bodies.

use crate::dialect::Dialect;
use crate::model::{StatementKind, StatementSpan};

/// Splits `sql` into statements. Empty statements (only whitespace/comments) are dropped.
pub fn split_statements(sql: &str, dialect: Dialect) -> Vec<StatementSpan> {
    match dialect {
        Dialect::Mongo => split_mongo(sql),
        _ => split_sql(sql, dialect),
    }
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

/// Kind and destructiveness of one statement's text (what `split_statements` stores per span).
pub fn classify_statement(sql: &str, dialect: Dialect) -> (StatementKind, bool) {
    match dialect {
        Dialect::Mongo => classify_mongo(sql),
        _ => classify_sql(sql, dialect),
    }
}

/// Kind of one statement's text.
pub fn statement_kind(sql: &str, dialect: Dialect) -> StatementKind {
    classify_statement(sql, dialect).0
}

/// A statement that only means something on one connection because it starts, ends or
/// subdivides a transaction, or changes how the connection's transactions behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxControl {
    /// BEGIN, START TRANSACTION.
    Begin,
    /// COMMIT, END.
    Commit,
    /// ROLLBACK, ABORT.
    Rollback,
    /// SAVEPOINT, RELEASE [SAVEPOINT], ROLLBACK TO [SAVEPOINT].
    Savepoint,
    /// SET autocommit, SET TRANSACTION.
    Settings,
}

/// The transaction control a statement performs, if any. Mongo scripts have none.
pub fn transaction_control(sql: &str, dialect: Dialect) -> Option<TxControl> {
    if dialect == Dialect::Mongo {
        return None;
    }
    let mut toks = Toks::new(sql, dialect).filter(|t| !matches!(t, Tok::Punct(b'@')));
    let Some(Tok::Word(first)) = toks.next() else {
        return None;
    };
    let mut next_is =
        |names: &[&str]| matches!(toks.next(), Some(Tok::Word(w)) if is_any_ci(w, names));
    if is_any_ci(first, &["BEGIN"]) {
        Some(TxControl::Begin)
    } else if is_any_ci(first, &["START"]) {
        next_is(&["TRANSACTION"]).then_some(TxControl::Begin)
    } else if is_any_ci(first, &["COMMIT", "END"]) {
        Some(TxControl::Commit)
    } else if is_any_ci(first, &["ABORT"]) {
        Some(TxControl::Rollback)
    } else if is_any_ci(first, &["SAVEPOINT", "RELEASE"]) {
        Some(TxControl::Savepoint)
    } else if is_any_ci(first, &["ROLLBACK"]) {
        // `ROLLBACK [WORK | TRANSACTION] TO [SAVEPOINT] name` keeps the transaction open.
        let mut rest = toks.filter_map(|t| match t {
            Tok::Word(w) => Some(w),
            _ => None,
        });
        let word = rest
            .by_ref()
            .find(|w| !is_any_ci(w, &["WORK", "TRANSACTION"]));
        Some(match word {
            Some(w) if is_any_ci(w, &["TO"]) => TxControl::Savepoint,
            _ => TxControl::Rollback,
        })
    } else if is_any_ci(first, &["SET"]) {
        // `SET [SESSION | LOCAL | GLOBAL] [@@]autocommit = …`, `SET [SESSION] TRANSACTION …`.
        let word = toks.find_map(|t| match t {
            Tok::Word(w) if !is_any_ci(w, &["SESSION", "LOCAL", "GLOBAL"]) => Some(Some(w)),
            Tok::Word(_) => None,
            _ => Some(None),
        });
        match word {
            Some(Some(w)) if is_any_ci(w, &["AUTOCOMMIT", "TRANSACTION"]) => {
                Some(TxControl::Settings)
            }
            _ => None,
        }
    } else {
        None
    }
}

/// What carries over from one chunk of a script to the next: the MySQL `DELIMITER` in effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkState {
    delimiter: Vec<u8>,
}

impl Default for ChunkState {
    fn default() -> Self {
        ChunkState {
            delimiter: vec![b';'],
        }
    }
}

/// Splits a script that is read in pieces (a dump too big to hold twice in memory).
///
/// `chunk` is the unprocessed text so far. Returns the statements that are complete (offsets
/// relative to `chunk`) and how many bytes of `chunk` they and the text before them used up; the
/// caller keeps the rest, appends the next piece and calls again with the same `state`. A
/// statement counts as complete once its terminator has been seen, so one cut off by the end of
/// the chunk is never returned half-read. With `last` set the chunk is the end of the script and
/// a final statement without terminator is returned too (everything is then consumed).
pub fn split_chunk(
    chunk: &str,
    dialect: Dialect,
    state: &mut ChunkState,
    last: bool,
) -> (Vec<StatementSpan>, usize) {
    match dialect {
        Dialect::Mongo => (split_mongo(chunk), chunk.len()),
        _ => split_sql_from(chunk, dialect, &mut state.delimiter, last),
    }
}

// ---------------------------------------------------------------------------
// Lexical helpers (byte level; every delimiter we look for is ASCII, so offsets
// always land on UTF-8 boundaries)
// ---------------------------------------------------------------------------

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

/// `i` is at the opening quote. Returns the index after the closing quote (or the end).
/// A doubled quote is an escaped quote; `backslash` makes `\x` skip a character.
fn skip_quoted(b: &[u8], i: usize, quote: u8, backslash: bool) -> usize {
    let mut i = i + 1;
    while i < b.len() {
        let c = b[i];
        if backslash && c == b'\\' {
            i += 2;
        } else if c == quote {
            if b.get(i + 1) == Some(&quote) {
                i += 2;
            } else {
                return i + 1;
            }
        } else {
            i += 1;
        }
    }
    b.len()
}

/// Returns the index of the terminating newline (or the end); the newline itself is not consumed.
fn skip_line(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

/// `i` is at `/*`. Postgres block comments nest.
fn skip_block(b: &[u8], i: usize, nest: bool) -> usize {
    let mut depth = 1;
    let mut i = i + 2;
    while i < b.len() {
        if nest && b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    b.len()
}

/// `i` is at a `$` that does not continue an identifier. Returns the index after the
/// closing tag when this opens a dollar-quoted string (`$$` or `$tag$`).
fn skip_dollar(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if j < b.len() && b[j].is_ascii_digit() {
        return None; // $1 parameter
    }
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] >= 0x80) {
        j += 1;
    }
    if b.get(j) != Some(&b'$') {
        return None;
    }
    let tag = &b[i..=j];
    let mut k = j + 1;
    while k + tag.len() <= b.len() {
        if b[k] == b'$' && &b[k..k + tag.len()] == tag {
            return Some(k + tag.len());
        }
        k += 1;
    }
    Some(b.len())
}

/// True when the quote at `i` is preceded by a lone `E`/`e` (Postgres escape string).
fn is_escape_string(b: &[u8], i: usize) -> bool {
    i >= 1 && matches!(b[i - 1], b'e' | b'E') && (i == 1 || !is_ident_char(b[i - 2]))
}

fn starts_with_ci(b: &[u8], word: &[u8]) -> bool {
    b.len() >= word.len() && b[..word.len()].eq_ignore_ascii_case(word)
}

/// Whether a `--` at `i` starts a comment. MySQL needs whitespace after the dashes.
fn dash_comment(b: &[u8], i: usize, mysql: bool) -> bool {
    b[i] == b'-'
        && b.get(i + 1) == Some(&b'-')
        && (!mysql
            || b.get(i + 2)
                .is_none_or(|&c| c.is_ascii_whitespace() || c < 0x20))
}

/// The identifier-ish word starting at the first non-blank character at or after `i`.
fn peek_word(b: &[u8], mut i: usize) -> Option<&[u8]> {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    let s = i;
    while i < b.len() && is_ident_char(b[i]) {
        i += 1;
    }
    (i > s).then(|| &b[s..i])
}

// ---------------------------------------------------------------------------
// SQL splitter
// ---------------------------------------------------------------------------

/// Per-statement lexical state: what kind of statement this is decides whether
/// semicolons inside BEGIN … END bodies terminate it.
#[derive(Default)]
struct StmtState {
    start: Option<usize>,
    /// End of the last significant (non-comment) token.
    last_end: usize,
    words: usize,
    is_create: bool,
    /// Some(true) for CREATE TRIGGER/PROCEDURE/FUNCTION/EVENT.
    routine: Option<bool>,
    depth: i32,
}

impl StmtState {
    /// Feeds one word; returns true when the following word is consumed too (`END CASE`).
    fn word(&mut self, w: &[u8], next: impl FnOnce() -> Option<Vec<u8>>) -> bool {
        const NOT_ROUTINE: [&str; 12] = [
            "TABLE",
            "VIEW",
            "INDEX",
            "DATABASE",
            "SCHEMA",
            "SEQUENCE",
            "TYPE",
            "DOMAIN",
            "EXTENSION",
            "ROLE",
            "USER",
            "VIRTUAL",
        ];
        const ROUTINE: [&str; 4] = ["TRIGGER", "PROCEDURE", "FUNCTION", "EVENT"];
        let is = |names: &[&str]| names.iter().any(|n| w.eq_ignore_ascii_case(n.as_bytes()));
        if self.words < 10 {
            self.words += 1;
            if self.words == 1 {
                self.is_create = w.eq_ignore_ascii_case(b"CREATE");
            } else if self.is_create && self.routine.is_none() {
                if is(&NOT_ROUTINE) {
                    self.routine = Some(false);
                } else if is(&ROUTINE) {
                    self.routine = Some(true);
                }
            }
        }
        if self.routine != Some(true) {
            return false;
        }
        if is(&["BEGIN", "CASE"]) {
            self.depth += 1;
        } else if w.eq_ignore_ascii_case(b"END") {
            let mut skip = false;
            match next() {
                // Compound statements that were not opened by BEGIN/CASE.
                Some(n) if is_any_ci(&n, &["IF", "LOOP", "WHILE", "REPEAT"]) => {}
                Some(n) if n.eq_ignore_ascii_case(b"CASE") => {
                    self.depth -= 1;
                    skip = true;
                }
                _ => self.depth -= 1,
            }
            self.depth = self.depth.max(0);
            return skip;
        }
        false
    }
}

fn is_any_ci(w: &[u8], names: &[&str]) -> bool {
    names.iter().any(|n| w.eq_ignore_ascii_case(n.as_bytes()))
}

fn split_sql(sql: &str, dialect: Dialect) -> Vec<StatementSpan> {
    split_sql_from(sql, dialect, &mut vec![b';'], true).0
}

/// The splitter proper. `last` false: statements without terminator are left for the caller to
/// complete with more text; the returned count says how much of `sql` has been dealt with.
fn split_sql_from(
    sql: &str,
    dialect: Dialect,
    delimiter: &mut Vec<u8>,
    last: bool,
) -> (Vec<StatementSpan>, usize) {
    let b = sql.as_bytes();
    let n = b.len();
    let mysql = matches!(dialect, Dialect::Mysql { .. });
    let pg = matches!(dialect, Dialect::Postgres);
    let sqlite = matches!(dialect, Dialect::Sqlite);
    let backslash = matches!(
        dialect,
        Dialect::Mysql {
            backslash_escapes: true
        }
    );

    let mut out = Vec::new();
    let mut consumed = 0;
    let mut st = StmtState::default();
    let mut line_start = true;
    let mut i = 0;

    let finish = |st: &mut StmtState, out: &mut Vec<StatementSpan>| {
        if let Some(start) = st.start.take()
            && st.last_end > start
        {
            let text = &sql[start..st.last_end];
            let (kind, destructive) = classify_sql(text, dialect);
            out.push(StatementSpan {
                start,
                end: st.last_end,
                sql: text.to_owned(),
                kind,
                destructive,
            });
        }
        *st = StmtState::default();
    };

    while i < n {
        let c = b[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Comments (not part of the statement text unless they are MySQL executable comments).
        if dash_comment(b, i, mysql) || (mysql && c == b'#') {
            i = skip_line(b, i);
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = skip_block(b, i, pg);
            if mysql && matches!(b.get(i + 2), Some(b'!' | b'+')) {
                st.start.get_or_insert(i);
                st.last_end = end;
            }
            i = end;
            line_start = false;
            continue;
        }

        // `DELIMITER xx` client command (MySQL), only between statements.
        if mysql
            && st.start.is_none()
            && line_start
            && starts_with_ci(&b[i..], b"DELIMITER")
            && matches!(b.get(i + 9), Some(b' ' | b'\t'))
        {
            let mut j = i + 9;
            while j < n && matches!(b[j], b' ' | b'\t') {
                j += 1;
            }
            let ds = j;
            while j < n && !b[j].is_ascii_whitespace() {
                j += 1;
            }
            let line_end = skip_line(b, j);
            if !last && line_end >= n {
                // The line may continue in the next piece.
                break;
            }
            if j > ds {
                *delimiter = b[ds..j].to_vec();
            }
            i = line_end;
            consumed = i;
            continue;
        }
        line_start = false;

        // Terminator. Semicolons inside BEGIN … END bodies belong to the body.
        if c == delimiter[0]
            && b[i..].starts_with(delimiter)
            && (delimiter.as_slice() != b";" || st.depth <= 0)
        {
            finish(&mut st, &mut out);
            i += delimiter.len();
            consumed = i;
            continue;
        }

        st.start.get_or_insert(i);
        match c {
            b'\'' => {
                i = skip_quoted(b, i, b'\'', backslash || (pg && is_escape_string(b, i)));
            }
            b'"' => i = skip_quoted(b, i, b'"', backslash),
            b'`' if !pg => i = skip_quoted(b, i, b'`', false),
            b'[' if sqlite => {
                i += 1;
                while i < n && b[i] != b']' {
                    i += 1;
                }
                i = (i + 1).min(n);
            }
            b'$' if pg && (i == 0 || !is_ident_char(b[i - 1])) => match skip_dollar(b, i) {
                Some(end) => i = end,
                None => i += 1,
            },
            c if is_ident_start(c) || c.is_ascii_digit() => {
                let s = i;
                let custom = delimiter.as_slice() != b";";
                // A custom delimiter such as `$$` may be glued to the last word.
                while i < n
                    && is_ident_char(b[i])
                    && !(custom && i > s && b[i..].starts_with(delimiter))
                {
                    i += 1;
                }
                let word = &b[s..i];
                if st.word(word, || peek_word(b, i).map(<[u8]>::to_vec)) {
                    // `END CASE`: swallow CASE.
                    while i < n && b[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    while i < n && is_ident_char(b[i]) {
                        i += 1;
                    }
                }
            }
            _ => i += 1,
        }
        st.last_end = i;
    }
    if last {
        finish(&mut st, &mut out);
        consumed = n;
    }
    (out, consumed)
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Tok<'a> {
    Word(&'a [u8]),
    Open,
    Close,
    Punct(u8),
    /// A string literal or quoted identifier.
    Quoted,
}

/// Lazy tokenizer over one statement: words, parentheses and punctuation, with
/// strings, quoted identifiers and comments reduced to `Quoted` / skipped.
struct Toks<'a> {
    b: &'a [u8],
    i: usize,
    dialect: Dialect,
}

impl<'a> Toks<'a> {
    fn new(sql: &'a str, dialect: Dialect) -> Self {
        Toks {
            b: sql.as_bytes(),
            i: 0,
            dialect,
        }
    }
}

impl<'a> Iterator for Toks<'a> {
    type Item = Tok<'a>;

    fn next(&mut self) -> Option<Tok<'a>> {
        let b = self.b;
        let mysql = matches!(self.dialect, Dialect::Mysql { .. });
        let pg = matches!(self.dialect, Dialect::Postgres);
        let backslash = matches!(
            self.dialect,
            Dialect::Mysql {
                backslash_escapes: true
            }
        );
        loop {
            let i = self.i;
            let c = *b.get(i)?;
            if c.is_ascii_whitespace() {
                self.i += 1;
            } else if dash_comment(b, i, mysql) || (mysql && c == b'#') {
                self.i = skip_line(b, i);
            } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
                self.i = skip_block(b, i, pg);
            } else if c == b'\'' {
                self.i = skip_quoted(b, i, b'\'', backslash || (pg && is_escape_string(b, i)));
                return Some(Tok::Quoted);
            } else if c == b'"' {
                self.i = skip_quoted(b, i, b'"', backslash);
                return Some(Tok::Quoted);
            } else if c == b'`' && !pg {
                self.i = skip_quoted(b, i, b'`', false);
                return Some(Tok::Quoted);
            } else if c == b'[' && matches!(self.dialect, Dialect::Sqlite) {
                let mut j = i + 1;
                while j < b.len() && b[j] != b']' {
                    j += 1;
                }
                self.i = (j + 1).min(b.len());
                return Some(Tok::Quoted);
            } else if c == b'$' && pg && (i == 0 || !is_ident_char(b[i - 1])) {
                if let Some(end) = skip_dollar(b, i) {
                    self.i = end;
                    return Some(Tok::Quoted);
                }
                self.i += 1;
                return Some(Tok::Punct(c));
            } else if is_ident_start(c) || c.is_ascii_digit() {
                let mut j = i;
                while j < b.len() && is_ident_char(b[j]) {
                    j += 1;
                }
                self.i = j;
                return Some(Tok::Word(&b[i..j]));
            } else {
                self.i += 1;
                return Some(match c {
                    b'(' => Tok::Open,
                    b')' => Tok::Close,
                    c => Tok::Punct(c),
                });
            }
        }
    }
}

fn classify_sql(sql: &str, dialect: Dialect) -> (StatementKind, bool) {
    let mut toks = Toks::new(sql, dialect);
    let first = loop {
        match toks.next() {
            Some(Tok::Open) => continue,
            Some(Tok::Word(w)) => break w,
            _ => return (StatementKind::Other, false),
        }
    };
    let is = |names: &[&str]| is_any_ci(first, names);

    if is(&[
        "SELECT", "SHOW", "EXPLAIN", "DESCRIBE", "DESC", "VALUES", "TABLE",
    ]) {
        return (StatementKind::Read, false);
    }
    if is(&["WITH"]) {
        return classify_with(toks);
    }
    if is(&["PRAGMA"]) {
        // `PRAGMA name` and `PRAGMA name(arg)` read; `PRAGMA name = value` changes a setting.
        let mut depth = 0;
        for t in toks {
            match t {
                Tok::Open => depth += 1,
                Tok::Close => depth -= 1,
                Tok::Punct(b'=') if depth == 0 => return (StatementKind::Other, false),
                _ => {}
            }
        }
        return (StatementKind::Read, false);
    }
    if is(&["DELETE", "UPDATE"]) {
        return (StatementKind::Write, !has_top_level_where(toks));
    }
    if is(&[
        "INSERT", "MERGE", "REPLACE", "UPSERT", "COPY", "CALL", "LOAD",
    ]) {
        return (StatementKind::Write, false);
    }
    if is(&["DROP", "TRUNCATE"]) {
        return (StatementKind::Ddl, true);
    }
    if is(&["ALTER"]) {
        return (StatementKind::Ddl, alter_drops(toks));
    }
    if is(&["CREATE", "RENAME", "GRANT", "REVOKE", "COMMENT"]) {
        return (StatementKind::Ddl, false);
    }
    (StatementKind::Other, false)
}

/// After `WITH`: the CTE definitions sit inside parentheses, so the first
/// top-level data keyword is the statement's real verb.
fn classify_with(mut toks: Toks<'_>) -> (StatementKind, bool) {
    let mut depth = 0;
    while let Some(t) = toks.next() {
        match t {
            Tok::Open => depth += 1,
            Tok::Close => depth -= 1,
            Tok::Word(w) if depth == 0 => {
                if is_any_ci(w, &["SELECT", "VALUES", "TABLE"]) {
                    return (StatementKind::Read, false);
                }
                if is_any_ci(w, &["DELETE", "UPDATE"]) {
                    return (StatementKind::Write, !has_top_level_where(toks));
                }
                if is_any_ci(w, &["INSERT", "MERGE", "REPLACE"]) {
                    return (StatementKind::Write, false);
                }
            }
            _ => {}
        }
    }
    (StatementKind::Read, false)
}

fn has_top_level_where(toks: Toks<'_>) -> bool {
    let mut depth = 0;
    for t in toks {
        match t {
            Tok::Open => depth += 1,
            Tok::Close => depth -= 1,
            Tok::Word(w) if depth == 0 && w.eq_ignore_ascii_case(b"WHERE") => return true,
            _ => {}
        }
    }
    false
}

/// `ALTER … DROP COLUMN/CONSTRAINT/INDEX …` loses data; `DROP DEFAULT`/`DROP NOT NULL` do not.
fn alter_drops(toks: Toks<'_>) -> bool {
    let mut after_drop = false;
    for t in toks {
        if let Tok::Word(w) = t {
            if after_drop && !is_any_ci(w, &["DEFAULT", "NOT", "IDENTITY", "EXPRESSION"]) {
                return true;
            }
            after_drop = w.eq_ignore_ascii_case(b"DROP");
        } else {
            after_drop = false;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Mongo shell
// ---------------------------------------------------------------------------

fn split_mongo(sql: &str) -> Vec<StatementSpan> {
    let b = sql.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut last_end = 0;
    let mut last_sig = 0u8;
    let mut depth = 0i32;
    let mut i = 0;

    let finish = |start: &mut Option<usize>, last_end: usize, out: &mut Vec<StatementSpan>| {
        if let Some(s) = start.take()
            && last_end > s
        {
            let text = &sql[s..last_end];
            let (kind, destructive) = classify_mongo(text);
            out.push(StatementSpan {
                start: s,
                end: last_end,
                sql: text.to_owned(),
                kind,
                destructive,
            });
        }
    };

    while i < n {
        let c = b[i];
        if c == b'\n' {
            if start.is_some() && depth <= 0 && !continues(last_sig) && !next_line_chains(b, i + 1)
            {
                finish(&mut start, last_end, &mut out);
                depth = 0;
                last_sig = 0;
            }
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            i = skip_line(b, i);
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i = skip_block(b, i, false);
            continue;
        }
        if c == b';' && depth <= 0 {
            finish(&mut start, last_end, &mut out);
            depth = 0;
            last_sig = 0;
            i += 1;
            continue;
        }
        start.get_or_insert(i);
        match c {
            b'\'' | b'"' | b'`' => i = skip_quoted(b, i, c, true),
            b'/' if regex_allowed(last_sig) => i = skip_regex(b, i),
            b'(' | b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b')' | b'}' | b']' => {
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
        last_end = i;
        last_sig = b[i - 1];
    }
    finish(&mut start, last_end, &mut out);
    out
}

/// A statement ending in one of these cannot be complete.
fn continues(last: u8) -> bool {
    matches!(
        last,
        b',' | b'.' | b'=' | b'+' | b'*' | b'%' | b'&' | b'|' | b'?' | b':' | b'<' | b'>'
    )
}

/// True when the next non-blank line starts with `.` (a chained call).
fn next_line_chains(b: &[u8], mut i: usize) -> bool {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    b.get(i) == Some(&b'.')
}

/// A `/` after one of these starts a regex literal rather than a division.
fn regex_allowed(last: u8) -> bool {
    matches!(
        last,
        0 | b'(' | b',' | b':' | b'[' | b'=' | b'{' | b'!' | b'&' | b'|' | b'?'
    )
}

/// `i` is at the opening `/` of a regex literal; returns the index after its flags.
fn skip_regex(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    let mut in_class = false;
    while j < b.len() && b[j] != b'\n' {
        match b[j] {
            b'\\' => j += 1,
            b'[' => in_class = true,
            b']' => in_class = false,
            b'/' if !in_class => {
                j += 1;
                while j < b.len() && b[j].is_ascii_alphabetic() {
                    j += 1;
                }
                return j;
            }
            _ => {}
        }
        j += 1;
    }
    j.min(b.len())
}

/// Method names (outside strings) in call order: `db.users.find({}).sort(…)` → find, sort.
fn mongo_methods(sql: &str) -> Vec<(String, usize)> {
    let b = sql.as_bytes();
    let mut methods = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' => i = skip_quoted(b, i, b[i], true),
            b'/' if b.get(i + 1) == Some(&b'/') => i = skip_line(b, i),
            b'/' if b.get(i + 1) == Some(&b'*') => i = skip_block(b, i, false),
            b'.' => {
                let s = i + 1;
                let mut j = s;
                while j < b.len() && is_ident_char(b[j]) {
                    j += 1;
                }
                let mut k = j;
                while k < b.len() && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                if j > s && b.get(k) == Some(&b'(') {
                    methods.push((sql[s..j].to_owned(), k));
                }
                i = j.max(i + 1);
            }
            _ => i += 1,
        }
    }
    methods
}

/// Text of the first argument of the call whose `(` is at `open`, trimmed; empty when none.
fn first_arg(sql: &str, open: usize) -> String {
    let b = sql.as_bytes();
    let mut depth = 0;
    let mut i = open;
    let mut start = open + 1;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' => {
                i = skip_quoted(b, i, b[i], true);
                continue;
            }
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return sql[start..i].trim().to_owned();
                }
            }
            b',' if depth == 1 => return sql[start..i].trim().to_owned(),
            _ => {}
        }
        i += 1;
        if depth == 0 {
            start = i;
        }
    }
    sql[start.min(sql.len())..].trim().to_owned()
}

fn classify_mongo(sql: &str) -> (StatementKind, bool) {
    let trimmed = sql.trim_start();
    let lower_head: String = trimmed
        .chars()
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase();
    if lower_head.starts_with("show ") {
        return (StatementKind::Read, false);
    }
    if lower_head.starts_with("use ") {
        return (StatementKind::Other, false);
    }

    const READ: [&str; 11] = [
        "find",
        "findone",
        "aggregate",
        "countdocuments",
        "count",
        "distinct",
        "estimateddocumentcount",
        "getindexes",
        "stats",
        "explain",
        "watch",
    ];
    const WRITE: [&str; 16] = [
        "insertone",
        "insertmany",
        "insert",
        "updateone",
        "updatemany",
        "update",
        "replaceone",
        "deleteone",
        "deletemany",
        "remove",
        "bulkwrite",
        "findoneandupdate",
        "findoneanddelete",
        "findoneandreplace",
        "save",
        "findandmodify",
    ];
    const DDL: [&str; 10] = [
        "createcollection",
        "createindex",
        "createindexes",
        "dropindex",
        "dropindexes",
        "drop",
        "dropdatabase",
        "renamecollection",
        "createview",
        "converttocapped",
    ];

    let mut kind = StatementKind::Other;
    let mut destructive = false;
    let mut kind_found = false;
    for (name, open) in mongo_methods(sql) {
        let lower = name.to_ascii_lowercase();
        let l = lower.as_str();
        if !kind_found {
            let k = if READ.contains(&l) {
                Some(StatementKind::Read)
            } else if WRITE.contains(&l) {
                Some(StatementKind::Write)
            } else if DDL.contains(&l) {
                Some(StatementKind::Ddl)
            } else if l == "runcommand" || l == "admincommand" {
                Some(StatementKind::Other)
            } else {
                None
            };
            if let Some(k) = k {
                kind = k;
                kind_found = true;
            }
        }
        match l {
            "drop" | "dropdatabase" => destructive = true,
            "deletemany" | "remove" | "updatemany" => {
                let arg = first_arg(sql, open);
                let compact: String = arg.chars().filter(|c| !c.is_whitespace()).collect();
                if compact.is_empty() || compact == "{}" {
                    destructive = true;
                }
            }
            _ => {}
        }
    }
    (kind, destructive)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PG: Dialect = Dialect::Postgres;
    const MY: Dialect = Dialect::Mysql {
        backslash_escapes: true,
    };
    const MY_NB: Dialect = Dialect::Mysql {
        backslash_escapes: false,
    };
    const LITE: Dialect = Dialect::Sqlite;

    fn texts(sql: &str, d: Dialect) -> Vec<String> {
        split_statements(sql, d)
            .into_iter()
            .map(|s| s.sql)
            .collect()
    }

    fn kinds(sql: &str, d: Dialect) -> Vec<(StatementKind, bool)> {
        split_statements(sql, d)
            .into_iter()
            .map(|s| (s.kind, s.destructive))
            .collect()
    }

    fn one(sql: &str, d: Dialect) -> (StatementKind, bool) {
        let k = kinds(sql, d);
        assert_eq!(k.len(), 1, "{sql}");
        k[0]
    }

    #[test]
    fn simple_statements() {
        assert_eq!(
            texts("select 1; select 2;\n\nselect 3", PG),
            ["select 1", "select 2", "select 3"]
        );
        assert!(split_statements("", PG).is_empty());
        assert!(split_statements("  ;; \n ; ", PG).is_empty());
    }

    #[test]
    fn semicolons_in_strings_and_identifiers() {
        assert_eq!(
            texts("select 'a;b'; select 2", PG),
            ["select 'a;b'", "select 2"]
        );
        assert_eq!(texts("select 'it''s;'; x", PG), ["select 'it''s;'", "x"]);
        assert_eq!(
            texts("select \"a;b\" from t; y", PG),
            ["select \"a;b\" from t", "y"]
        );
        assert_eq!(
            texts("select `a;b` from t; y", MY),
            ["select `a;b` from t", "y"]
        );
        assert_eq!(
            texts("select [a;b] from t; y", LITE),
            ["select [a;b] from t", "y"]
        );
    }

    #[test]
    fn backslash_escapes_depend_on_dialect() {
        // MySQL: \' does not end the string.
        assert_eq!(texts(r"select 'a\';b'; x", MY), [r"select 'a\';b'", "x"]);
        // NO_BACKSLASH_ESCAPES, Postgres and SQLite: the backslash is literal.
        assert_eq!(texts(r"select 'a\'; x", MY_NB), [r"select 'a\'", "x"]);
        assert_eq!(texts(r"select 'a\'; x", PG), [r"select 'a\'", "x"]);
        assert_eq!(texts(r"select 'a\'; x", LITE), [r"select 'a\'", "x"]);
        // Postgres escape strings do honour it.
        assert_eq!(texts(r"select E'a\';b'; x", PG), [r"select E'a\';b'", "x"]);
        assert_eq!(
            texts(r#"select "a\";b"; x"#, MY),
            [r#"select "a\";b""#, "x"]
        );
    }

    #[test]
    fn comments() {
        assert_eq!(
            texts("select 1 -- a; b\n; select 2", PG),
            ["select 1", "select 2"]
        );
        assert_eq!(texts("select /* ; */ 1; x", PG), ["select /* ; */ 1", "x"]);
        // Postgres block comments nest.
        assert_eq!(
            texts("select /* a /* ; */ ; */ 1; x", PG),
            ["select /* a /* ; */ ; */ 1", "x"]
        );
        // Other dialects do not.
        assert_eq!(
            texts("select /* a /* ; */ 1; x", MY),
            ["select /* a /* ; */ 1", "x"]
        );
        // MySQL # comments and the whitespace rule for --.
        assert_eq!(texts("select 1 # a;b\n; x", MY), ["select 1", "x"]);
        assert_eq!(texts("select 1--1; x", MY), ["select 1--1", "x"]);
        assert_eq!(texts("select 1-- 1; x", MY), ["select 1"]);
        // # is an operator elsewhere.
        assert_eq!(
            texts("select '{1}' #> '{a}'; x", PG),
            ["select '{1}' #> '{a}'", "x"]
        );
    }

    #[test]
    fn leading_and_trailing_comments_are_not_part_of_the_statement() {
        let sql = "-- first\nselect 1; -- trailing\n/* only a comment */\nselect 2 -- end";
        let spans = split_statements(sql, PG);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].sql, "select 1");
        assert_eq!(&sql[spans[0].start..spans[0].end], "select 1");
        assert_eq!(spans[1].sql, "select 2");
    }

    #[test]
    fn mysql_executable_comments_are_statements() {
        assert_eq!(
            texts("/*!40101 SET NAMES utf8 */;\nselect 1;", MY),
            ["/*!40101 SET NAMES utf8 */", "select 1"]
        );
    }

    #[test]
    fn dollar_quoting() {
        let f = "create function f() returns int as $$ begin select 1; return 2; end $$ language plpgsql";
        assert_eq!(texts(&format!("{f}; select 3"), PG), [f, "select 3"]);
        let tagged = "do $body$ begin perform 1; end $body$";
        assert_eq!(texts(&format!("{tagged}; x"), PG), [tagged, "x"]);
        // Nested different tags.
        let nested = "select $a$ x $b$ ; $b$ y $a$";
        assert_eq!(texts(&format!("{nested}; z"), PG), [nested, "z"]);
        // $1 parameters and identifiers containing $ are not quotes.
        assert_eq!(
            texts("select $1; select a$b$c", PG),
            ["select $1", "select a$b$c"]
        );
        // Not Postgres: $$ means nothing.
        assert_eq!(texts("select $$; select 2", MY), ["select $$", "select 2"]);
        // Unterminated dollar quote runs to the end.
        assert_eq!(texts("select $$ abc; def", PG), ["select $$ abc; def"]);
    }

    #[test]
    fn mysql_delimiter() {
        let sql = "DELIMITER //\n\
                   CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND//\n\
                   DELIMITER ;\n\
                   SELECT 3;";
        let t = texts(sql, MY);
        assert_eq!(t.len(), 2);
        assert!(t[0].starts_with("CREATE PROCEDURE p()"));
        assert!(t[0].ends_with("END"));
        assert!(t[0].contains("SELECT 1;"));
        assert_eq!(t[1], "SELECT 3");
        // Statements before and after, custom multi-char delimiter.
        assert_eq!(
            texts(
                "SELECT 1;\ndelimiter $$\nSELECT 2; SELECT 3$$\nDELIMITER ;\nSELECT 4;",
                MY
            ),
            ["SELECT 1", "SELECT 2; SELECT 3", "SELECT 4"]
        );
    }

    /// Splits `sql` the way a reader of a big file does: `size` bytes at a time (a piece may end
    /// inside a multi-byte character, so it is extended to one boundary).
    fn in_pieces(sql: &str, d: Dialect, size: usize) -> Vec<String> {
        let mut state = ChunkState::default();
        let mut out = Vec::new();
        let mut carry = String::new();
        let mut rest = sql;
        loop {
            let mut end = size.min(rest.len());
            while end < rest.len() && !rest.is_char_boundary(end) {
                end += 1;
            }
            carry.push_str(&rest[..end]);
            rest = &rest[end..];
            let last = rest.is_empty();
            let (spans, used) = split_chunk(&carry, d, &mut state, last);
            for span in &spans {
                assert_eq!(span.sql, carry[span.start..span.end]);
            }
            out.extend(spans.into_iter().map(|s| s.sql));
            carry.drain(..used);
            if last {
                assert!(carry.is_empty());
                return out;
            }
        }
    }

    #[test]
    fn pieces_split_like_the_whole() {
        let scripts: [(&str, Dialect); 7] = [
            (
                "SELECT 1;\nDELIMITER //\nCREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END//\n\
                 DELIMITER ;\nSELECT 'a;b' ; -- c;\nSELECT `x;y`, \"q;\";\n/*!40101 SET x=1 */;\n\
                 delimiter $$\nSELECT 'é'; SELECT 3$$\nDELIMITER ;\nSELECT 4",
                MY,
            ),
            (
                "SELECT 'it\\'s;', 2;\nSELECT 'ünï;';\n# hash; comment\nSELECT 3;",
                MY,
            ),
            (
                "SELECT $$a;b$$;\nCREATE FUNCTION f() RETURNS int AS $body$ BEGIN RETURN 1; END; $body$ LANGUAGE plpgsql;\n\
                 SELECT E'it\\'s;';\n/* a /* nested */ ; */ SELECT 2;\nSELECT 'ünï;'",
                PG,
            ),
            (
                "CREATE TRIGGER t AFTER INSERT ON x BEGIN UPDATE x SET a = 1; INSERT INTO y VALUES (2); END;\n\
                 CREATE TABLE [we;ird] (a);\nSELECT 1; SELECT 'x'",
                LITE,
            ),
            ("   \n-- only a comment\n/* and another */  ", LITE),
            ("", PG),
            ("SELECT 日本語🙂;SELECT 'é'", MY_NB),
        ];
        for (sql, d) in scripts {
            let whole = texts(sql, d);
            for size in (1..=sql.len().max(1)).chain([1 << 20]) {
                assert_eq!(
                    in_pieces(sql, d, size),
                    whole,
                    "{d:?}, {size} bytes at a time: {sql:?}"
                );
            }
        }
    }

    #[test]
    fn a_piece_leaves_unfinished_statements_for_the_next() {
        let mut state = ChunkState::default();
        let (spans, used) = split_chunk("SELECT 1; SELECT 2", PG, &mut state, false);
        assert_eq!(
            spans.iter().map(|s| s.sql.as_str()).collect::<Vec<_>>(),
            ["SELECT 1"]
        );
        assert_eq!(used, "SELECT 1;".len());
        // The tail is returned when it is known to be the end.
        let (spans, used) = split_chunk(" SELECT 2", PG, &mut state, true);
        assert_eq!(spans.len(), 1);
        assert_eq!(used, " SELECT 2".len());
        // Nothing complete: nothing consumed, and an open string is not a statement.
        let (spans, used) = split_chunk(
            "SELECT 'abc; SELECT 1;",
            PG,
            &mut ChunkState::default(),
            false,
        );
        assert!(spans.is_empty());
        assert_eq!(used, 0);
    }

    #[test]
    fn a_delimiter_command_cut_by_the_piece_waits_for_its_line_end() {
        let mut state = ChunkState::default();
        let (spans, used) = split_chunk("SELECT 1;\nDELIMITER /", MY, &mut state, false);
        assert_eq!(spans.len(), 1);
        assert_eq!(used, "SELECT 1;".len());
        // Once the line is complete the new delimiter is in effect, and it carries over.
        let (spans, used) = split_chunk(
            "\nDELIMITER //\nSELECT 2; SELECT 3//",
            MY,
            &mut state,
            false,
        );
        assert_eq!(
            spans.iter().map(|s| s.sql.as_str()).collect::<Vec<_>>(),
            ["SELECT 2; SELECT 3"]
        );
        assert_eq!(used, "\nDELIMITER //\nSELECT 2; SELECT 3//".len());
        let (spans, _) = split_chunk(
            "\nSELECT 4; SELECT 5//\nDELIMITER ;\n",
            MY,
            &mut state,
            true,
        );
        assert_eq!(
            spans.iter().map(|s| s.sql.as_str()).collect::<Vec<_>>(),
            ["SELECT 4; SELECT 5"]
        );
    }

    #[test]
    fn mysql_delimiter_is_ignored_elsewhere() {
        // Not at the start of a line / not MySQL: ordinary text.
        assert_eq!(texts("select delimiter; x", MY), ["select delimiter", "x"]);
        assert_eq!(texts("DELIMITER //\nx", PG), ["DELIMITER //\nx"]);
    }

    #[test]
    fn begin_end_bodies_without_delimiter() {
        let proc_ = "CREATE PROCEDURE p() BEGIN IF x THEN SELECT 1; END IF; SELECT 2; END";
        assert_eq!(
            texts(&format!("{proc_}; select 9"), MY),
            [proc_, "select 9"]
        );
        let case = "CREATE FUNCTION f(a INT) RETURNS INT BEGIN CASE a WHEN 1 THEN RETURN 1; ELSE RETURN 2; END CASE; END";
        assert_eq!(texts(&format!("{case}; x"), MY), [case, "x"]);
        let expr = "CREATE PROCEDURE p() BEGIN SELECT CASE WHEN a THEN 1 ELSE 2 END; SELECT 3; END";
        assert_eq!(texts(&format!("{expr}; x"), MY), [expr, "x"]);
        let lp = "CREATE PROCEDURE p() BEGIN loop1: LOOP SELECT 1; END LOOP; WHILE a DO SELECT 2; END WHILE; END";
        assert_eq!(texts(&format!("{lp}; x"), MY), [lp, "x"]);
    }

    #[test]
    fn sqlite_triggers() {
        let trig = "CREATE TRIGGER t AFTER INSERT ON a BEGIN INSERT INTO b VALUES (1); UPDATE c SET x = CASE WHEN 1 THEN 2 ELSE 3 END; END";
        assert_eq!(
            texts(&format!("{trig}; select 1; {trig};"), LITE),
            [trig, "select 1", trig]
        );
        // BEGIN as a transaction statement is not a body.
        assert_eq!(
            texts("BEGIN; select 1; COMMIT;", LITE),
            ["BEGIN", "select 1", "COMMIT"]
        );
        // A table called "trigger" does not start a body.
        assert_eq!(
            texts("CREATE TABLE trigger (a); select 1", LITE),
            ["CREATE TABLE trigger (a)", "select 1"]
        );
    }

    #[test]
    fn pg_sql_standard_function_body() {
        let f = "CREATE FUNCTION add(a int, b int) RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT a + b; END";
        assert_eq!(texts(&format!("{f}; x"), PG), [f, "x"]);
    }

    #[test]
    fn unicode_offsets_are_byte_offsets() {
        let sql = "select 'héllo'; select '日本語', 2 ;select 3";
        let spans = split_statements(sql, PG);
        assert_eq!(spans.len(), 3);
        for s in &spans {
            assert_eq!(&sql[s.start..s.end], s.sql);
        }
        assert_eq!(spans[1].sql, "select '日本語', 2");
        assert_eq!(spans[1].start, "select 'héllo'; ".len());
        assert_eq!(spans[1].end, sql.find(" ;").unwrap());
    }

    #[test]
    fn unicode_identifiers_start_a_statement() {
        let sql = "ünïcode; select 1";
        let spans = split_statements(sql, PG);
        assert_eq!(spans[0].sql, "ünïcode");
        assert_eq!(spans[1].sql, "select 1");
    }

    #[test]
    fn statement_at_cursor() {
        let sql = "select 1;\nselect 2;\nselect 3;";
        let d = PG;
        assert_eq!(statement_at(sql, d, 0).unwrap().sql, "select 1");
        assert_eq!(statement_at(sql, d, 14).unwrap().sql, "select 2");
        // Between statements: the one before.
        assert_eq!(statement_at(sql, d, 9).unwrap().sql, "select 1");
        assert_eq!(statement_at(sql, d, 1000).unwrap().sql, "select 3");
        assert!(statement_at("", d, 0).is_none());
    }

    #[test]
    fn kinds_of_statements() {
        use StatementKind::*;
        let cases: &[(&str, StatementKind)] = &[
            ("SELECT 1", Read),
            ("  (SELECT 1) UNION (SELECT 2)", Read),
            ("with a as (select 1) select * from a", Read),
            (
                "WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM a) SELECT * FROM a",
                Read,
            ),
            ("with a as (select 1) insert into t select * from a", Write),
            ("show tables", Read),
            ("EXPLAIN ANALYZE select 1", Read),
            ("describe t", Read),
            ("values (1), (2)", Read),
            ("table t", Read),
            ("PRAGMA table_info(t)", Read),
            ("PRAGMA foreign_keys = ON", Other),
            ("insert into t values (1)", Write),
            ("UPDATE t SET a = 1 WHERE b = 2", Write),
            ("delete from t where a = 1", Write),
            (
                "MERGE INTO t USING s ON 1=1 WHEN MATCHED THEN DELETE",
                Write,
            ),
            ("REPLACE INTO t VALUES (1)", Write),
            ("COPY t FROM stdin", Write),
            ("CALL p()", Write),
            ("create table t (a int)", Ddl),
            ("ALTER TABLE t ADD c int", Ddl),
            ("drop table t", Ddl),
            ("truncate t", Ddl),
            ("rename table a to b", Ddl),
            ("GRANT ALL ON t TO u", Ddl),
            ("REVOKE ALL ON t FROM u", Ddl),
            ("comment on table t is 'x'", Ddl),
            ("BEGIN", Other),
            ("COMMIT", Other),
            ("SET search_path = a", Other),
            ("USE db", Other),
            ("-- just a comment\nSELECT 1", Read),
        ];
        for (sql, kind) in cases {
            assert_eq!(statement_kind(sql, PG), *kind, "{sql}");
        }
    }

    #[test]
    fn destructive_statements() {
        let yes = [
            "DROP TABLE t",
            "drop database d",
            "TRUNCATE TABLE t",
            "ALTER TABLE t DROP COLUMN c",
            "alter table t drop constraint k",
            "ALTER TABLE t DROP INDEX i",
            "DELETE FROM t",
            "update t set a = 1",
            "UPDATE t SET a = (SELECT b FROM u WHERE u.id = 1)",
            "with x as (select 1 where true) delete from t",
            "DELETE FROM t -- WHERE a = 1",
            "DELETE FROM t /* where */",
            "DELETE FROM t WHERE_ = 1",
        ];
        for sql in yes {
            let (_, destructive) = one(sql, PG);
            // The last case has no real WHERE word either.
            assert!(destructive, "{sql}");
        }
        let no = [
            "DELETE FROM t WHERE a = 1",
            "update t set a = 1 where id = 2",
            "UPDATE t SET a = 1 FROM u WHERE t.id = u.id",
            "SELECT * FROM t",
            "INSERT INTO t SELECT * FROM u",
            "ALTER TABLE t ADD COLUMN c int",
            "ALTER TABLE t ALTER COLUMN c DROP NOT NULL",
            "ALTER TABLE t ALTER COLUMN c DROP DEFAULT",
            "CREATE TABLE t (a int)",
            "delete from t where a in (select id from u)",
            "select 'DROP TABLE t'",
            "-- DROP TABLE t\nselect 1",
        ];
        for sql in no {
            let (_, destructive) = one(sql, PG);
            assert!(!destructive, "{sql}");
        }
    }

    #[test]
    fn destructive_in_script_is_flagged_per_statement() {
        let k = kinds("select 1; delete from t; delete from t where a = 1;", MY);
        assert_eq!(
            k,
            [
                (StatementKind::Read, false),
                (StatementKind::Write, true),
                (StatementKind::Write, false)
            ]
        );
    }

    #[test]
    fn create_trigger_body_is_not_destructive() {
        let sql = "CREATE TRIGGER t AFTER DELETE ON a BEGIN DELETE FROM b; END";
        assert_eq!(one(sql, LITE), (StatementKind::Ddl, false));
    }

    // --- Mongo ---------------------------------------------------------------

    const MONGO: Dialect = Dialect::Mongo;

    #[test]
    fn mongo_semicolons_and_newlines() {
        assert_eq!(
            texts("db.a.find({}); db.b.find({})", MONGO),
            ["db.a.find({})", "db.b.find({})"]
        );
        assert_eq!(
            texts(
                "db.a.find({})\ndb.b.find({})\n\nshow collections\nuse mydb",
                MONGO
            ),
            [
                "db.a.find({})",
                "db.b.find({})",
                "show collections",
                "use mydb"
            ]
        );
    }

    #[test]
    fn mongo_multiline_literals_and_chains() {
        let multi = "db.users.find({\n  age: { $gt: 3 },\n  tags: [\n    'a',\n    'b'\n  ]\n})";
        assert_eq!(
            texts(&format!("{multi}\ndb.x.find()"), MONGO),
            [multi, "db.x.find()"]
        );
        let chained = "db.users.find({})\n  .sort({ a: 1 })\n  .limit(5)";
        assert_eq!(texts(chained, MONGO), [chained]);
        let tail = "db.a.aggregate([\n { $match: {} }\n],\n {allowDiskUse: true})";
        assert_eq!(texts(tail, MONGO), [tail]);
    }

    #[test]
    fn mongo_strings_comments_and_regexes() {
        assert_eq!(
            texts("db.a.find({ s: 'x;y\\'z' }); db.b.find()", MONGO),
            ["db.a.find({ s: 'x;y\\'z' })", "db.b.find()"]
        );
        assert_eq!(
            texts(
                "// hello\ndb.a.find() // trailing\n/* block\n;*/ db.b.find()",
                MONGO
            ),
            ["db.a.find()", "db.b.find()"]
        );
        // A quote inside a regex does not open a string.
        assert_eq!(
            texts("db.a.find({ n: /it's/i })\ndb.b.find()", MONGO),
            ["db.a.find({ n: /it's/i })", "db.b.find()"]
        );
    }

    #[test]
    fn mongo_trailing_operator_continues() {
        let sql = "var x = 1 +\n  2";
        assert_eq!(texts(sql, MONGO), [sql]);
    }

    #[test]
    fn mongo_kinds_and_destructive() {
        use StatementKind::*;
        let cases: &[(&str, StatementKind, bool)] = &[
            ("db.users.find({})", Read, false),
            ("db.users.find({}).sort({a:1})", Read, false),
            ("db.getCollection('users').find()", Read, false),
            ("db.users.aggregate([])", Read, false),
            ("db.users.countDocuments({})", Read, false),
            ("show dbs", Read, false),
            ("use mydb", Other, false),
            ("db.users.insertOne({a: 1})", Write, false),
            ("db.users.updateOne({a: 1}, {$set: {b: 2}})", Write, false),
            ("db.users.deleteOne({a: 1})", Write, false),
            ("db.users.deleteMany({a: 1})", Write, false),
            ("db.users.deleteMany({})", Write, true),
            ("db.users.deleteMany( { } )", Write, true),
            ("db.users.deleteMany()", Write, true),
            ("db.users.remove({})", Write, true),
            ("db.users.remove({ a: 1 })", Write, false),
            ("db.users.updateMany({}, {$set: {a: 1}})", Write, true),
            ("db.users.updateMany({a: 1}, {$set: {a: 2}})", Write, false),
            ("db.users.drop()", Ddl, true),
            ("db.dropDatabase()", Ddl, true),
            ("db.users.createIndex({a: 1})", Ddl, false),
            ("db.createCollection('x')", Ddl, false),
            (
                "db.users.insertOne({note: 'db.users.drop()'})",
                Write,
                false,
            ),
            ("db.runCommand({ping: 1})", Other, false),
        ];
        for (sql, kind, destructive) in cases {
            assert_eq!(
                classify_statement(sql, MONGO),
                (*kind, *destructive),
                "{sql}"
            );
        }
    }
}
