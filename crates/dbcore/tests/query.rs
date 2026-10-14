//! The query path: scripts, splitting, result sets, affected rows, errors, sticky
//! connections and transactions across statements.

#[macro_use]
mod common;

use std::time::{Duration, Instant};

use common::*;
use dbcore::driver::CursorEvent;
use dbcore::model::{StatementKind, TypeFamily};
use dbcore::split::split_statements;
use dbcore::{Engine, Error, Value};

fn vint(i: i64) -> Value {
    Value::Int(i)
}

fn vtext(s: &str) -> Value {
    Value::Text(s.to_owned())
}

fn int_type(c: &Ctx) -> &'static str {
    match c.engine {
        Engine::Postgres => "integer",
        Engine::Sqlite => "INTEGER",
        _ => "INT",
    }
}

fn text_type(c: &Ctx) -> &'static str {
    match c.engine {
        Engine::Postgres => "text",
        Engine::Sqlite => "TEXT",
        _ => "VARCHAR(200)",
    }
}

/// Runs a script and returns every statement's events, failing on the first error.
async fn script_ok(c: &Ctx, script: &str) -> Vec<(String, Vec<CursorEvent>)> {
    run_script(c, script)
        .await
        .into_iter()
        .map(|(sql, result)| match result {
            Ok(events) => (sql, events),
            Err(e) => panic!("statement failed: {e:?}\n  SQL: {sql}"),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Scripts and result shapes
// ---------------------------------------------------------------------------

async fn a_script_reports_each_statement(c: &Ctx) {
    let t = c.scratch("script");
    let table = c.qt(&t);
    let (int, text) = (int_type(c), text_type(c));
    let script = format!(
        "CREATE TABLE {table} (id {int} PRIMARY KEY, v {text});
         INSERT INTO {table} VALUES (1, 'a'), (2, 'b'), (3, 'c');
         SELECT id, v FROM {table} ORDER BY id;
         UPDATE {table} SET v = 'z' WHERE id < 3;
         UPDATE {table} SET v = 'nothing' WHERE id > 100;
         DELETE FROM {table} WHERE id = 3;
         DELETE FROM {table} WHERE id = 3;
         SELECT COUNT(*) AS n FROM {table};
         SELECT id FROM {table} WHERE id > 100;
         CREATE INDEX {ix} ON {table} (v);
         DROP INDEX {ix2};",
        ix = c.q(&c.name("script_ix")),
        ix2 = if c.is_my() {
            format!("{} ON {table}", c.q(&c.name("script_ix")))
        } else {
            c.q(&c.name("script_ix"))
        },
    );
    let results = script_ok(c, &script).await;
    assert_eq!(results.len(), 11);
    let e = |i: usize| &results[i].1;
    assert_eq!(affected_of(e(0)), [0], "CREATE TABLE");
    assert_eq!(e(0).len(), 1);
    assert_eq!(e(1), &[CursorEvent::Affected(3)], "INSERT of three rows");
    let select = sets_of(e(2));
    assert_eq!(select.len(), 1);
    assert_eq!(select[0].names(), ["id", "v"]);
    assert_eq!(
        select[0].rows,
        vec![
            vec![vint(1), vtext("a")],
            vec![vint(2), vtext("b")],
            vec![vint(3), vtext("c")]
        ]
    );
    assert_eq!(e(3), &[CursorEvent::Affected(2)], "UPDATE of two rows");
    assert_eq!(e(4), &[CursorEvent::Affected(0)], "UPDATE of nothing");
    assert_eq!(e(5), &[CursorEvent::Affected(1)], "DELETE of one row");
    assert_eq!(e(6), &[CursorEvent::Affected(0)], "DELETE of nothing");
    assert_eq!(sets_of(e(7))[0].names(), ["n"]);
    assert_eq!(sets_of(e(7))[0].rows, vec![vec![vint(2)]]);
    // A SELECT without rows still reports its columns, and no row count.
    assert_eq!(e(8).len(), 1, "{:?}", e(8));
    assert!(
        matches!(&e(8)[0], CursorEvent::Columns(cols) if cols.len() == 1 && cols[0].name == "id")
    );
    assert_eq!(
        affected_of(e(9)),
        [0],
        "CREATE INDEX never reports the earlier statement's count"
    );
    assert_eq!(affected_of(e(10)), [0]);
}
each_engine!(a_script_reports_each_statement, sql);

async fn empty_results_keep_their_columns(c: &Ctx) {
    // The kind of statement decides whether a result without rows still has columns, so
    // comments, parentheses and CTEs in front must not confuse it.
    let t = c.make_items("empty_sets", 3).await;
    let table = c.qt(&t);
    let selects = [
        format!("SELECT id, name FROM {table} WHERE 1 = 0"),
        format!("  \n\t SELECT id, name FROM {table} WHERE 1 = 0"),
        format!("/* leading comment */ SELECT id, name FROM {table} WHERE 1 = 0"),
        format!("-- line comment\nSELECT id, name FROM {table} WHERE 1 = 0"),
        if c.is_sqlite() {
            // SQLite does not accept a parenthesised query at the top level.
            format!("SELECT id, name FROM (SELECT id, name FROM {table}) WHERE 1 = 0")
        } else {
            format!("(SELECT id, name FROM {table} WHERE 1 = 0)")
        },
        format!("WITH q AS (SELECT id, name FROM {table}) SELECT id, name FROM q WHERE 1 = 0"),
        format!("select ID, NAME from {table} where 1 = 0"),
    ];
    for sql in selects {
        let sql = if c.is_my() {
            sql.replace("-- line", "-- line ")
        } else {
            sql
        };
        let events = c.run(&sql).await;
        assert_eq!(events.len(), 1, "{sql:?}: {events:?}");
        let CursorEvent::Columns(cols) = &events[0] else {
            panic!("{sql:?}: {events:?}")
        };
        assert_eq!(cols.len(), 2, "{sql:?}");
        assert_eq!(cols[0].family, TypeFamily::Integer, "{sql:?}");
        assert_eq!(cols[1].family, TypeFamily::Text, "{sql:?}");
    }
    // Statements that only change data report a count even when nothing matched.
    assert_eq!(
        c.run(&format!("UPDATE {table} SET grp = 1 WHERE 1 = 0"))
            .await,
        [CursorEvent::Affected(0)]
    );
    assert_eq!(
        c.run(&format!("/* c */ DELETE FROM {table} WHERE 1 = 0"))
            .await,
        [CursorEvent::Affected(0)]
    );
    // VALUES lists and table expressions are queries too.
    let values = match c.engine {
        Engine::Mysql | Engine::Mariadb if c.is_mariadb() => "VALUES (1), (2)",
        Engine::Mysql => "VALUES ROW(1), ROW(2)",
        _ => "VALUES (1), (2)",
    };
    let sets = sets_of(&c.run(values).await);
    assert_eq!(sets[0].rows.len(), 2);
}
each_engine!(empty_results_keep_their_columns, sql);

async fn duplicate_column_names_keep_every_value(c: &Ctx) {
    let q = |a: &str| c.q(a);
    let events = c
        .run(&format!(
            "SELECT 1 AS {a}, 2 AS {a}, 3 AS {u}, 'x' AS {a}",
            a = q("a"),
            u = q("ünï")
        ))
        .await;
    let set = &sets_of(&events)[0];
    assert_eq!(set.names(), ["a", "a", "ünï", "a"]);
    assert_eq!(set.rows[0], vec![vint(1), vint(2), vint(3), vtext("x")]);

    // The same name from two tables of a join.
    let (int_ty, text_ty) = (int_type(c), text_type(c));
    let (p, ch) = (c.scratch("dup_p"), c.scratch("dup_c"));
    c.side_exec(&format!(
        "CREATE TABLE {} (id {int_ty} PRIMARY KEY, name {text_ty})",
        c.qt(&p)
    ))
    .await;
    c.side_exec(&format!(
        "CREATE TABLE {} (id {int_ty} PRIMARY KEY, name {text_ty}, p {int_ty})",
        c.qt(&ch)
    ))
    .await;
    c.side_exec(&format!("INSERT INTO {} VALUES (1, 'parent')", c.qt(&p)))
        .await;
    c.side_exec(&format!("INSERT INTO {} VALUES (7, 'child', 1)", c.qt(&ch)))
        .await;
    let sql = format!(
        "SELECT p.id, ch.id, p.name, ch.name FROM {} p JOIN {} ch ON ch.p = p.id",
        c.qt(&p),
        c.qt(&ch)
    );
    let set = &sets_of(&c.run(&sql).await)[0];
    assert_eq!(set.names(), ["id", "id", "name", "name"]);
    assert_eq!(
        set.rows[0],
        vec![vint(1), vint(7), vtext("parent"), vtext("child")]
    );
    let star = format!(
        "SELECT * FROM {} p JOIN {} ch ON ch.p = p.id",
        c.qt(&p),
        c.qt(&ch)
    );
    let set = &sets_of(&c.run(&star).await)[0];
    assert_eq!(set.names(), ["id", "name", "id", "name", "p"]);
    assert_eq!(set.rows[0].len(), 5);
    assert_eq!(set.rows[0][2], vint(7));

    // Unnamed expressions get the engine's own names.
    let set = &sets_of(&c.run("SELECT 1, 2").await)[0];
    assert_eq!(set.columns.len(), 2);
    assert_eq!(set.rows[0], vec![vint(1), vint(2)]);
    match c.engine {
        Engine::Postgres => assert_eq!(set.names(), ["?column?", "?column?"]),
        _ => assert_eq!(set.names(), ["1", "2"]),
    }
    // The same table browsed or exported keeps working with those columns.
    let big = format!(
        "SELECT {} FROM {}",
        (0..40)
            .map(|i| format!("{i} AS c{}", i % 3))
            .collect::<Vec<_>>()
            .join(", "),
        c.qt(&p)
    );
    let set = &sets_of(&c.run(&big).await)[0];
    assert_eq!(set.columns.len(), 40);
    assert_eq!(set.rows[0].len(), 40);
    assert_eq!(set.rows[0][39], vint(39));
}
each_engine!(duplicate_column_names_keep_every_value, sql);

async fn introspection_statements_return_result_sets(c: &Ctx) {
    let t = c.make_items("introspect", 3).await;
    let table = c.qt(&t);
    let statements: Vec<(String, Vec<&str>)> = match c.engine {
        Engine::Postgres => vec![
            ("SHOW server_version".into(), vec![]),
            (format!("EXPLAIN SELECT * FROM {table}"), vec!["QUERY PLAN"]),
            ("SELECT version()".into(), vec!["version"]),
            (
                format!("SELECT * FROM information_schema.columns WHERE table_name = '{t}'"),
                vec![],
            ),
        ],
        Engine::Sqlite => vec![
            (
                format!("PRAGMA table_info({})", c.q(&t)),
                vec!["cid", "name", "type", "notnull", "dflt_value", "pk"],
            ),
            (
                format!("EXPLAIN QUERY PLAN SELECT * FROM {table}"),
                vec!["id", "parent", "notused", "detail"],
            ),
            ("PRAGMA user_version".into(), vec!["user_version"]),
            ("SELECT sqlite_version()".into(), vec![]),
        ],
        _ => vec![
            ("SHOW TABLES".into(), vec![]),
            (
                format!("DESCRIBE {table}"),
                vec!["Field", "Type", "Null", "Key", "Default", "Extra"],
            ),
            (
                format!("SHOW CREATE TABLE {table}"),
                vec!["Table", "Create Table"],
            ),
            (
                "SHOW VARIABLES LIKE 'sql_mode'".into(),
                vec!["Variable_name", "Value"],
            ),
            (format!("EXPLAIN SELECT * FROM {table}"), vec![]),
            ("SELECT VERSION()".into(), vec![]),
        ],
    };
    for (sql, names) in statements {
        let events = c.run(&sql).await;
        let sets = sets_of(&events);
        assert_eq!(sets.len(), 1, "{sql}: {events:?}");
        assert!(!sets[0].rows.is_empty(), "{sql} returned no rows");
        if !names.is_empty() {
            assert_eq!(sets[0].names(), names, "{sql}");
        }
    }
    let describe = sets_of(&c.run(&match c.engine {
        Engine::Sqlite => format!("PRAGMA table_info({})", c.q(&t)),
        Engine::Postgres => format!("SELECT column_name FROM information_schema.columns WHERE table_name = '{t}' ORDER BY ordinal_position"),
        _ => format!("DESCRIBE {table}"),
    }).await);
    assert_eq!(describe[0].rows.len(), 4);
}
each_engine!(introspection_statements_return_result_sets, sql);

async fn dml_with_returning_streams_rows(c: &Ctx) {
    let (int, text) = (int_type(c), text_type(c));
    let t = c.scratch("returning");
    let table = c.qt(&t);
    c.side_exec(&format!(
        "CREATE TABLE {table} (id {int} PRIMARY KEY, v {text})"
    ))
    .await;
    let events = c
        .run(&format!(
            "INSERT INTO {table} VALUES (1, 'a'), (2, 'b') RETURNING id, v"
        ))
        .await;
    let set = &sets_of(&events)[0];
    assert_eq!(set.names(), ["id", "v"]);
    assert_eq!(
        set.rows,
        vec![vec![vint(1), vtext("a")], vec![vint(2), vtext("b")]]
    );
    let events = c
        .run(&format!("DELETE FROM {table} WHERE id = 1 RETURNING v"))
        .await;
    assert_eq!(sets_of(&events)[0].rows, vec![vec![vtext("a")]]);
    assert_eq!(c.side_count(&t).await, 1);
    // A CTE that writes.
    if c.is_pg() {
        let events = c
            .run(&format!(
                "WITH moved AS (DELETE FROM {table} WHERE id = 2 RETURNING id) SELECT id FROM moved"
            ))
            .await;
        assert_eq!(sets_of(&events)[0].rows, vec![vec![vint(2)]]);
        assert_eq!(c.side_count(&t).await, 0);
    }
}
each_engine!(dml_with_returning_streams_rows, postgres, sqlite, mariadb);

// ---------------------------------------------------------------------------
// Splitting scripts, then running what came out
// ---------------------------------------------------------------------------

async fn tricky_scripts_split_and_run(c: &Ctx) {
    let t = c.scratch("split");
    let table = c.qt(&t);
    let (int, text) = (int_type(c), text_type(c));
    let hash_comment = if c.is_my() {
        "# a hash comment; with a semicolon\n"
    } else {
        ""
    };
    let dash = if c.is_my() { "-- " } else { "--" };
    let script = format!(
        "CREATE TABLE {table} (id {int} PRIMARY KEY, v {text});
         {dash} comment; with a semicolon
         INSERT INTO {table} VALUES (1, 'a;b'), (2, ';'), (3, '-- not a comment'), (4, '/* nor this */');
         {hash_comment}/* block; comment; */ INSERT INTO {table} VALUES (5, 'after block');
         ;;
         INSERT INTO {table} VALUES (6, 'it''s; fine');
         INSERT INTO {table} VALUES (7, 'tab\there'), (8, '日本語; 🎉');
         INSERT INTO {table} VALUES (9, 'line one
line two; still the same string');
         INSERT INTO {table} VALUES (10, 'no terminator')"
    );
    let spans = split_statements(&script, c.dialect);
    assert_eq!(
        spans.len(),
        7,
        "{:#?}",
        spans.iter().map(|s| &s.sql).collect::<Vec<_>>()
    );
    for span in &spans {
        assert_eq!(
            &script[span.start..span.end],
            span.sql,
            "offsets must index the script"
        );
    }
    assert_eq!(spans[0].kind, StatementKind::Ddl);
    assert!(spans[1..].iter().all(|s| s.kind == StatementKind::Write));
    let results = script_ok(c, &script).await;
    assert_eq!(results.len(), 7);
    let rows = c
        .rows(&format!("SELECT id, v FROM {table} ORDER BY id"))
        .await;
    assert_eq!(rows.len(), 10);
    let v = |i: usize| rows[i - 1][1].clone();
    assert_eq!(v(1), vtext("a;b"));
    assert_eq!(v(2), vtext(";"));
    assert_eq!(v(3), vtext("-- not a comment"));
    assert_eq!(v(4), vtext("/* nor this */"));
    assert_eq!(v(5), vtext("after block"));
    assert_eq!(v(6), vtext("it's; fine"));
    assert_eq!(v(8), vtext("日本語; 🎉"));
    assert_eq!(v(9), vtext("line one\nline two; still the same string"));
    assert_eq!(v(10), vtext("no terminator"));
}
each_engine!(tricky_scripts_split_and_run, sql);

async fn quoted_identifiers_with_semicolons(c: &Ctx) {
    let name = format!("{}_semi;colon \"x\" -- y", c.prefix);
    c.cleanup_sql(format!("DROP TABLE IF EXISTS {}", c.qt(&name)));
    let (int, text) = (int_type(c), text_type(c));
    let script = format!(
        "CREATE TABLE {t} ({a} {int}, {b} {text}); INSERT INTO {t} ({a}, {b}) VALUES (1, 'x'); SELECT {a}, {b} FROM {t}",
        t = c.qt(&name),
        a = c.q("a;b"),
        b = c.q("c -- d"),
    );
    let spans = split_statements(&script, c.dialect);
    assert_eq!(spans.len(), 3);
    let results = script_ok(c, &script).await;
    let set = &sets_of(&results[2].1)[0];
    assert_eq!(set.names(), ["a;b", "c -- d"]);
    assert_eq!(set.rows, vec![vec![vint(1), vtext("x")]]);
}
each_engine!(quoted_identifiers_with_semicolons, sql);

async fn pg_dollar_quoting_and_functions(c: &Ctx) {
    let t = c.scratch("dollar");
    let table = c.qt(&t);
    let func = c.q(&c.name("fn"));
    c.cleanup_sql(format!("DROP FUNCTION IF EXISTS {func}(integer)"));
    let script = format!(
        "CREATE TABLE {table} (id integer PRIMARY KEY, v text);
         INSERT INTO {table} VALUES (1, $$one; two$$), (2, $tag$ $$ nested; $$ $tag$), (3, E'it\\'s; escaped');
         /* outer /* nested; comment */ still comment; */ INSERT INTO {table} VALUES (4, 'x');
         CREATE FUNCTION {func}(n integer) RETURNS integer LANGUAGE plpgsql AS $body$
         DECLARE total integer := 0;
         BEGIN
             FOR i IN 1..n LOOP total := total + i; END LOOP;
             INSERT INTO {table} VALUES (100, 'from function; ' || total);
             RETURN total;
         END;
         $body$;
         SELECT {func}(4) AS total;
         DO $$ BEGIN INSERT INTO {table} VALUES (200, 'from do block; ok'); END $$;
         SELECT id, v FROM {table} ORDER BY id"
    );
    let spans = split_statements(&script, c.dialect);
    assert_eq!(
        spans.len(),
        7,
        "{:#?}",
        spans.iter().map(|s| &s.sql).collect::<Vec<_>>()
    );
    let results = script_ok(c, &script).await;
    assert_eq!(sets_of(&results[4].1)[0].rows, vec![vec![vint(10)]]);
    let rows = &sets_of(&results[6].1)[0].rows;
    let values: Vec<(i64, String)> = rows
        .iter()
        .map(|r| (ids(std::slice::from_ref(r), 0)[0], r[1].to_display()))
        .collect();
    assert_eq!(
        values,
        vec![
            (1, "one; two".to_owned()),
            (2, " $$ nested; $$ ".to_owned()),
            (3, "it's; escaped".to_owned()),
            (4, "x".to_owned()),
            (100, "from function; 10".to_owned()),
            (200, "from do block; ok".to_owned()),
        ]
    );
}
each_engine!(pg_dollar_quoting_and_functions, postgres);

async fn mysql_delimiter_procedures_and_multiple_result_sets(c: &Ctx) {
    let t = c.scratch("proc");
    let table = c.qt(&t);
    let proc_name = c.q(&c.name("proc"));
    c.cleanup_sql(format!("DROP PROCEDURE IF EXISTS {}.{proc_name}", c.q(DB)));
    let script = format!(
        "CREATE TABLE {table} (id INT PRIMARY KEY, v VARCHAR(50));
         DELIMITER //
         CREATE PROCEDURE {proc_name}(IN n INT)
         BEGIN
             DECLARE i INT DEFAULT 1;
             WHILE i <= n DO
                 INSERT INTO {table} VALUES (i, CONCAT('row; ', i));
                 SET i = i + 1;
             END WHILE;
             SELECT COUNT(*) AS inserted FROM {table};
             SELECT id, v FROM {table} ORDER BY id DESC;
         END //
         DELIMITER ;
         CALL {proc_name}(3);
         SELECT 'after the procedure; done' AS note"
    );
    let spans = split_statements(&script, c.dialect);
    assert_eq!(
        spans.len(),
        4,
        "{:#?}",
        spans.iter().map(|s| &s.sql).collect::<Vec<_>>()
    );
    assert!(
        spans[1].sql.starts_with("CREATE PROCEDURE") && spans[1].sql.ends_with("END"),
        "{}",
        spans[1].sql
    );
    let results = script_ok(c, &script).await;
    // CALL returns two result sets.
    let sets = sets_of(&results[2].1);
    assert_eq!(sets.len(), 2, "{:?}", results[2].1);
    assert_eq!(sets[0].names(), ["inserted"]);
    assert_eq!(sets[0].rows, vec![vec![vint(3)]]);
    assert_eq!(sets[1].names(), ["id", "v"]);
    assert_eq!(sets[1].rows.len(), 3);
    assert_eq!(sets[1].rows[0], vec![vint(3), vtext("row; 3")]);
    assert_eq!(
        sets_of(&results[3].1)[0].rows,
        vec![vec![vtext("after the procedure; done")]]
    );
    // Conditional comments and # comments.
    let script =
        "SELECT 1 AS a; # a comment; with semicolons\n/*!40101 SET @x = 1 */; SELECT @x AS x";
    let spans = split_statements(script, c.dialect);
    assert_eq!(spans.len(), 3);
    let results = script_ok(c, script).await;
    assert_eq!(
        sets_of(&results[2].1)[0].rows,
        vec![vec![vint(1)]],
        "@x survives between statements"
    );
}
each_engine!(
    mysql_delimiter_procedures_and_multiple_result_sets,
    mysql,
    mariadb
);

async fn mysql_backslash_escapes_in_strings(c: &Ctx) {
    let results = script_ok(
        c,
        "SELECT 'it\\'s; ok' AS a; SELECT 'back\\\\slash; ' AS b; SELECT \"dq; \\\"inner\\\"\" AS c; SELECT 'a''b; c' AS d",
    )
    .await;
    assert_eq!(results.len(), 4);
    let cell = |i: usize| sets_of(&results[i].1)[0].rows[0][0].clone();
    assert_eq!(cell(0), vtext("it's; ok"));
    assert_eq!(cell(1), vtext("back\\slash; "));
    assert_eq!(cell(2), vtext("dq; \"inner\""));
    assert_eq!(cell(3), vtext("a'b; c"));
}
each_engine!(mysql_backslash_escapes_in_strings, mysql, mariadb);

async fn sqlite_triggers_and_virtual_tables(c: &Ctx) {
    let (a, b, log) = (c.scratch("trg_a"), c.scratch("trg_b"), c.scratch("trg_log"));
    let trig = c.q(&c.name("trg"));
    let script = format!(
        "CREATE TABLE {a} (id INTEGER PRIMARY KEY, v TEXT);
         CREATE TABLE {b} (id INTEGER PRIMARY KEY, v TEXT);
         CREATE TABLE {log} (msg TEXT);
         CREATE TRIGGER {trig} AFTER INSERT ON {a}
         BEGIN
             INSERT INTO {log} VALUES ('inserted; ' || NEW.v);
             INSERT INTO {b} VALUES (NEW.id, NEW.v);
             SELECT CASE WHEN NEW.v = 'bad' THEN RAISE(ABORT, 'no bad values; sorry') END;
         END;
         INSERT INTO {a} VALUES (1, 'good');
         SELECT msg FROM {log};",
        a = c.q(&a),
        b = c.q(&b),
        log = c.q(&log),
    );
    let spans = split_statements(&script, c.dialect);
    assert_eq!(
        spans.len(),
        6,
        "{:#?}",
        spans.iter().map(|s| &s.sql).collect::<Vec<_>>()
    );
    let results = script_ok(c, &script).await;
    assert_eq!(
        sets_of(&results[5].1)[0].rows,
        vec![vec![vtext("inserted; good")]]
    );
    // The trigger can veto a statement; the error text survives.
    let err = c
        .events(&format!("INSERT INTO {} VALUES (2, 'bad')", c.q(&a)))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Database { message, .. } if message.contains("no bad values; sorry")),
        "{err:?}"
    );
    assert_eq!(c.side_count(&a).await, 1);
}
each_engine!(sqlite_triggers_and_virtual_tables, sqlite);

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

async fn errors_carry_message_and_code(c: &Ctx) {
    let t = c.make_items("errors", 3).await;
    let table = c.qt(&t);
    struct Case {
        sql: String,
        message: &'static str,
        pg: &'static str,
        mysql: &'static str,
    }
    let cases = [
        Case {
            sql: "SELEC 1".into(),
            message: "SELEC",
            pg: "42601",
            mysql: "42000",
        },
        Case {
            sql: "SELECT FROM WHERE".into(),
            message: "",
            pg: "42601",
            mysql: "42000",
        },
        Case {
            sql: "SELECT * FROM no_such_table_xyz".into(),
            message: "no_such_table_xyz",
            pg: "42P01",
            mysql: "42S02",
        },
        Case {
            sql: format!("SELECT no_such_column FROM {table}"),
            message: "no_such_column",
            pg: "42703",
            mysql: "42S22",
        },
        Case {
            sql: format!("INSERT INTO {table} (id, name, grp) VALUES (1, 'dup', 1)"),
            message: "",
            pg: "23505",
            mysql: "23000",
        },
        Case {
            sql: format!("INSERT INTO {table} (id, name, grp) VALUES (50, NULL, 1)"),
            message: "",
            pg: "23502",
            mysql: "23000",
        },
        Case {
            sql: format!("SELECT * FROM {table} WHERE id = 'x' ||"),
            message: "",
            pg: "42601",
            mysql: "42000",
        },
    ];
    for case in cases {
        let err = c
            .events(&case.sql)
            .await
            .expect_err(&format!("{} should fail", case.sql));
        let Error::Database { message, code, .. } = &err else {
            panic!("{}: {err:?}", case.sql)
        };
        assert!(!message.is_empty(), "{}", case.sql);
        assert!(message.contains(case.message), "{}: {message}", case.sql);
        assert_eq!(err.kind(), "database");
        match c.engine {
            Engine::Postgres => {
                assert_eq!(code.as_deref(), Some(case.pg), "{}: {message}", case.sql)
            }
            Engine::Mysql | Engine::Mariadb => {
                assert_eq!(code.as_deref(), Some(case.mysql), "{}: {message}", case.sql)
            }
            _ => assert!(code.is_some(), "{}: {message}", case.sql),
        }
        // The error serializes the way the UI reads it.
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "database");
        assert_eq!(json["message"], serde_json::Value::String(message.clone()));
        assert!(json["code"].is_string());
        // The connection is fine afterwards.
        assert_eq!(c.scalar("SELECT 1").await, vint(1));
    }
    // A script stops at its first error, leaving the earlier statements applied.
    let results = run_script(
        c,
        &format!("UPDATE {table} SET grp = 42 WHERE id = 1; SELECT * FROM nope_xyz; UPDATE {table} SET grp = 43 WHERE id = 2"),
    )
    .await;
    assert_eq!(results.len(), 2);
    assert!(results[0].1.is_ok() && results[1].1.is_err());
    assert_eq!(
        c.rows(&format!(
            "SELECT id, grp FROM {table} WHERE id <= 2 ORDER BY id"
        ))
        .await,
        vec![vec![vint(1), vint(42)], vec![vint(2), vint(2)]]
    );
}
each_engine!(errors_carry_message_and_code, sql);

async fn errors_in_the_middle_of_a_result(c: &Ctx) {
    // The first rows are good, then the server fails: the cursor reports the error.
    let sql = match c.engine {
        Engine::Postgres => "SELECT 10 / (x - 3) AS q FROM generate_series(1, 6) AS x",
        Engine::Sqlite => {
            "SELECT CASE WHEN x = 4 THEN abs(-9223372036854775807 - 1) ELSE x END FROM (SELECT 1 AS x UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4)"
        }
        _ => {
            "SELECT CAST(CONCAT('1', x) AS SIGNED) / (3 - x) AS q FROM (SELECT 1 AS x UNION ALL SELECT 2 UNION ALL SELECT 3) t"
        }
    };
    if c.is_sqlite() || c.is_my() {
        // These engines do not fail on this query (SQLite has no such error; MySQL yields NULL).
        let events = c.run(sql).await;
        assert!(!sets_of(&events).is_empty());
        return;
    }
    let mut cursor = c.driver.query(None, sql).await.unwrap();
    let mut rows = 0;
    let err = loop {
        match cursor.next(100).await {
            Ok(Some(CursorEvent::Rows(r))) => rows += r.len(),
            Ok(Some(_)) => {}
            Ok(None) => panic!("the query should have failed"),
            Err(e) => break e,
        }
    };
    assert!(
        matches!(&err, Error::Database { code: Some(code), .. } if code == "22012"),
        "{err:?}"
    );
    assert!(rows <= 2, "rows before the failing one: {rows}");
    // Nothing is left half-open.
    assert_eq!(c.scalar("SELECT 2").await, vint(2));
}
each_engine!(errors_in_the_middle_of_a_result, postgres);

// ---------------------------------------------------------------------------
// Sticky connections and transactions
// ---------------------------------------------------------------------------

fn begin(c: &Ctx) -> &'static str {
    if c.is_my() {
        "START TRANSACTION"
    } else {
        "BEGIN"
    }
}

async fn transactions_span_statements(c: &Ctx) {
    let t = c.make_items("tx", 3).await;
    let table = c.qt(&t);
    let count = || c.scalar(format!("SELECT COUNT(*) FROM {table}").leak());
    let extra = |id: i64| format!("INSERT INTO {table} (id, name, grp) VALUES ({id}, 'tx', 1)");

    // ROLLBACK undoes what the earlier statements did.
    c.run(begin(c)).await;
    c.run(&extra(10)).await;
    assert_eq!(
        count().await,
        vint(4),
        "the transaction sees its own insert"
    );
    assert_eq!(c.side_count(&t).await, 3, "other connections do not");
    c.run("ROLLBACK").await;
    assert_eq!(count().await, vint(3));

    // COMMIT makes it permanent.
    c.run(begin(c)).await;
    c.run(&extra(11)).await;
    c.run(&format!("UPDATE {table} SET grp = 99 WHERE id = 1"))
        .await;
    c.run("COMMIT").await;
    assert_eq!(c.side_count(&t).await, 4);
    assert_eq!(
        c.side_text(&format!("SELECT grp FROM {table} WHERE id = 1"))
            .await
            .as_deref(),
        Some("99")
    );

    // Savepoints.
    c.run(begin(c)).await;
    c.run(&extra(12)).await;
    c.run("SAVEPOINT sp1").await;
    c.run(&extra(13)).await;
    assert_eq!(count().await, vint(6));
    c.run("ROLLBACK TO SAVEPOINT sp1").await;
    assert_eq!(count().await, vint(5));
    c.run("COMMIT").await;
    assert_eq!(c.side_count(&t).await, 5);
    let ids_now = ids(
        &c.rows(&format!("SELECT id FROM {table} ORDER BY id")).await,
        0,
    );
    assert_eq!(ids_now, [1, 2, 3, 11, 12]);
}
each_engine!(transactions_span_statements, sql);

async fn session_state_persists_between_statements(c: &Ctx) {
    match c.engine {
        Engine::Postgres => {
            c.run("SET application_name = 'tablesail-sticky'").await;
            assert_eq!(
                c.scalar("SHOW application_name").await,
                vtext("tablesail-sticky")
            );
            c.run("SET search_path = billing, public").await;
            assert_eq!(c.scalar("SHOW search_path").await, vtext("billing, public"));
            assert_eq!(
                c.scalar("SELECT count(*) >= 0 FROM invoices").await,
                Value::Bool(true)
            );
            c.run("RESET search_path").await;
            c.run("CREATE TEMP TABLE sticky_tmp (a int)").await;
            c.run("INSERT INTO sticky_tmp VALUES (1), (2)").await;
            assert_eq!(c.scalar("SELECT count(*) FROM sticky_tmp").await, vint(2));
            c.run("DROP TABLE sticky_tmp").await;
        }
        Engine::Sqlite => {
            c.run("PRAGMA foreign_keys = OFF").await;
            assert_eq!(c.scalar("PRAGMA foreign_keys").await, vint(0));
            c.run("PRAGMA foreign_keys = ON").await;
            assert_eq!(c.scalar("PRAGMA foreign_keys").await, vint(1));
            c.run("CREATE TEMP TABLE sticky_tmp (a)").await;
            c.run("INSERT INTO sticky_tmp VALUES (1), (2)").await;
            assert_eq!(c.scalar("SELECT count(*) FROM sticky_tmp").await, vint(2));
        }
        _ => {
            c.run("SET @sticky = 41").await;
            assert_eq!(c.scalar("SELECT @sticky + 1").await, vint(42));
            c.run("SET SESSION sql_mode = 'ANSI_QUOTES'").await;
            assert_eq!(
                c.scalar("SELECT @@SESSION.sql_mode").await,
                vtext("ANSI_QUOTES")
            );
            c.run("SET SESSION sql_mode = DEFAULT").await;
            c.run("CREATE TEMPORARY TABLE sticky_tmp (a INT)").await;
            c.run("INSERT INTO sticky_tmp VALUES (1), (2)").await;
            assert_eq!(c.scalar("SELECT COUNT(*) FROM sticky_tmp").await, vint(2));
            // A script's own USE stays in effect for its later statements.
            c.run("USE tablesail_other").await;
            assert_eq!(
                c.scalar("SELECT DATABASE()").await,
                vtext("tablesail_other")
            );
            c.run("USE tablesail").await;
            assert_eq!(c.scalar("SELECT DATABASE()").await, vtext("tablesail"));
        }
    }
}
each_engine!(session_state_persists_between_statements, sql);

async fn a_failed_statement_inside_a_transaction(c: &Ctx) {
    let t = c.make_items("tx_fail", 2).await;
    let table = c.qt(&t);
    c.run("BEGIN").await;
    c.run(&format!(
        "INSERT INTO {table} (id, name, grp) VALUES (10, 'x', 1)"
    ))
    .await;
    let err = c.events("SELECT 1 / 0").await.unwrap_err();
    assert!(
        matches!(&err, Error::Database { code: Some(code), .. } if code == "22012"),
        "{err:?}"
    );
    // Postgres refuses everything else until the transaction ends.
    let err = c.events("SELECT 1").await.unwrap_err();
    assert!(
        matches!(&err, Error::Database { code: Some(code), .. } if code == "25P02"),
        "{err:?}"
    );
    c.run("ROLLBACK").await;
    assert_eq!(c.scalar("SELECT 1").await, vint(1));
    assert_eq!(c.side_count(&t).await, 2, "the insert was rolled back");
}
each_engine!(a_failed_statement_inside_a_transaction, postgres);

async fn uncommitted_work_is_rolled_back_when_the_driver_closes(c: &Ctx) {
    let t = c.make_items("tx_close", 2).await;
    let table = c.qt(&t);
    let driver = connect_driver(c.engine, c.file.as_deref(), |_| {})
        .await
        .unwrap();
    let run = |sql: String| {
        let driver = driver.clone();
        async move {
            drain(driver.query(Some(DB), &sql).await.unwrap())
                .await
                .unwrap()
        }
    };
    run(begin(c).to_owned()).await;
    run(format!(
        "INSERT INTO {table} (id, name, grp) VALUES (10, 'x', 1)"
    ))
    .await;
    run(format!("UPDATE {table} SET grp = 99 WHERE id = 1")).await;
    driver.close().await;
    eventually(
        Duration::from_secs(5),
        "the server rolls back the abandoned transaction",
        || async {
            c.side_count(&t).await == 2
                && c.side_text(&format!("SELECT grp FROM {table} WHERE id = 1"))
                    .await
                    .as_deref()
                    != Some("99")
        },
    )
    .await;
}
each_engine!(uncommitted_work_is_rolled_back_when_the_driver_closes, sql);

async fn concurrent_statements_use_separate_connections(c: &Ctx) {
    let t = c.make_items("conc", 3).await;
    let table = c.qt(&t);
    // A transaction is open on the sticky connection...
    c.run(begin(c)).await;
    c.run(&format!("UPDATE {table} SET grp = 7 WHERE id = 1"))
        .await;
    // ...while a long-running query holds that connection busy; a new statement goes
    // to another one and sees the committed state, not the transaction's.
    let slow = match c.engine {
        Engine::Postgres => "SELECT pg_sleep(1)",
        Engine::Sqlite => {
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 3000000) SELECT count(*) FROM c"
        }
        _ => "SELECT SLEEP(1)",
    };
    let mut slow_cursor = c.driver.query(Some(&c.database), slow).await.unwrap();
    let started = Instant::now();
    let other = c
        .scalar(&format!("SELECT grp FROM {table} WHERE id = 1"))
        .await;
    if !c.is_sqlite() {
        assert_eq!(
            other,
            vint(1),
            "a second connection does not see the open transaction"
        );
        assert!(
            started.elapsed() < Duration::from_millis(900),
            "it did not wait for the slow query"
        );
    }
    while slow_cursor.next(10).await.unwrap().is_some() {}
    drop(slow_cursor);
    c.run("ROLLBACK").await;
    assert_eq!(
        c.scalar(&format!("SELECT grp FROM {table} WHERE id = 1"))
            .await,
        vint(1)
    );
}
each_engine!(concurrent_statements_use_separate_connections, sql);

async fn other_databases_are_reachable_through_query(c: &Ctx) {
    match c.engine {
        Engine::Postgres => {
            let events = drain(
                c.driver
                    .query(Some("postgres"), "SELECT current_database()")
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(sets_of(&events)[0].rows, vec![vec![vtext("postgres")]]);
            assert_eq!(
                c.scalar("SELECT current_database()").await,
                vtext("tablesail")
            );
            // Tables of another database are not visible from this one.
            let err = drain(
                c.driver
                    .query(Some("postgres"), "SELECT * FROM all_types")
                    .await
                    .unwrap(),
            )
            .await
            .unwrap_err();
            assert!(matches!(err, Error::Database { .. }));
        }
        _ => {
            let other = drain(
                c.driver
                    .query(Some("tablesail_other"), "SELECT DATABASE()")
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(
                sets_of(&other)[0].rows,
                vec![vec![vtext("tablesail_other")]]
            );
            let back = drain(
                c.driver
                    .query(Some("tablesail"), "SELECT DATABASE()")
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(sets_of(&back)[0].rows, vec![vec![vtext("tablesail")]]);
            let notes = drain(
                c.driver
                    .query(Some("tablesail_other"), "SELECT COUNT(*) FROM notes")
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(sets_of(&notes)[0].rows.len(), 1);
            let err = c
                .driver
                .query(Some("no_such_database_xyz"), "SELECT 1")
                .await;
            match err {
                Err(e) => assert!(matches!(e, Error::Database { .. }), "{e:?}"),
                Ok(cursor) => assert!(drain(cursor).await.is_err()),
            }
            assert_eq!(c.scalar("SELECT DATABASE()").await, vtext("tablesail"));
        }
    }
}
each_engine!(
    other_databases_are_reachable_through_query,
    postgres,
    mysql,
    mariadb
);

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

async fn large_results_stream_in_bounded_batches(c: &Ctx) {
    let n: i64 = 200_000;
    let sql = match c.engine {
        Engine::Postgres => {
            format!("SELECT g AS n, 'row ' || g AS label FROM generate_series(1, {n}) AS g")
        }
        Engine::Sqlite => format!(
            "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < {n}) SELECT n, 'row ' || n AS label FROM s"
        ),
        _ => format!(
            "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < {n}) SELECT n, CONCAT('row ', n) AS label FROM s"
        ),
    };
    let sql = if c.is_my() {
        // MySQL caps recursion at 1000 by default; raise it for this session only.
        if c.is_mariadb() {
            c.run("SET SESSION max_recursive_iterations = 4294967295")
                .await;
        } else {
            c.run("SET SESSION cte_max_recursion_depth = 1000000").await;
        }
        sql
    } else {
        sql
    };
    let started = Instant::now();
    let mut cursor = c.driver.query(Some(&c.database), &sql).await.unwrap();
    let Some(CursorEvent::Columns(cols)) = cursor.next(500).await.unwrap() else {
        panic!("the first event is the column list")
    };
    assert_eq!(cols.len(), 2);
    let mut total = 0i64;
    let mut batches = 0;
    let mut expected_next = 1;
    let mut first_row_at = None;
    while let Some(event) = cursor.next(500).await.unwrap() {
        let CursorEvent::Rows(rows) = event else {
            continue;
        };
        first_row_at.get_or_insert_with(|| started.elapsed());
        assert!(
            rows.len() <= 500,
            "batches honour the requested size: {}",
            rows.len()
        );
        for row in &rows {
            assert_eq!(row[0], vint(expected_next));
            expected_next += 1;
        }
        total += rows.len() as i64;
        batches += 1;
    }
    assert_eq!(total, n);
    assert!(batches >= n / 500);
    assert!(first_row_at.unwrap() < Duration::from_secs(2));
    drop(cursor);
    assert_eq!(c.scalar("SELECT 1").await, vint(1));
}
each_engine!(large_results_stream_in_bounded_batches, sql);

async fn export_cursors_deliver_complete_values(c: &Ctx) {
    let t = c.make_items("browse_all", 2500).await;
    let mut cursor = c.driver.browse_all(&c.browse(&t)).await.unwrap();
    let mut rows = Vec::new();
    let mut columns = None;
    while let Some(event) = cursor.next(1000).await.unwrap() {
        match event {
            CursorEvent::Columns(cols) => {
                assert!(columns.replace(cols).is_none(), "a single result set");
            }
            CursorEvent::Rows(batch) => {
                assert!(batch.len() <= 1000);
                rows.extend(batch);
            }
            CursorEvent::Affected(_) => panic!("a table read has no row count"),
        }
    }
    assert_eq!(columns.unwrap().len(), 4);
    assert_eq!(rows.len(), 2500);
    rows.sort_by_key(|r| ids(std::slice::from_ref(r), 0)[0]);
    assert_eq!(rows, (1..=2500).map(item_row).collect::<Vec<_>>());
    // Filters, sorting and projection apply to the export too.
    let mut req = c.browse(&t);
    req.columns = vec!["id".into()];
    req.filters = vec![filter("grp", dbcore::model::FilterOp::Eq, Some("3"))];
    req.sort = vec![sort("id", true)];
    let got = ids(&c.all_rows(&req).await, 0);
    let mut want: Vec<i64> = (1..=2500).filter(|i| item(*i).grp == 3).collect();
    want.reverse();
    assert_eq!(got, want);
}
each_engine!(export_cursors_deliver_complete_values, sql);

// ---------------------------------------------------------------------------
// Schema changes made through the editor
// ---------------------------------------------------------------------------

async fn ddl_through_the_editor_refreshes_cached_structures(c: &Ctx) {
    use dbcore::model::{ChangeSet, ColumnInput, ColumnValue, RowChange};
    use dbcore::value::CellInput;
    let t = c.make_items("ddl_cache", 3).await;
    let table = c.qt(&t);
    // The grid has loaded the structure (and with it the editability and columns).
    let before = c.driver.table_structure(&c.tref(&t)).await.unwrap();
    assert_eq!(before.columns.len(), 4);
    let _ = c.page(&c.browse(&t), 0, 10).await;

    // A column is added in the SQL editor; editing it works straight away.
    let text = text_type(c);
    c.run(&format!("ALTER TABLE {table} ADD COLUMN extra {text}"))
        .await;
    let edit = ChangeSet {
        table: c.tref(&t),
        changes: vec![RowChange::Update {
            key: vec![ColumnValue {
                column: "id".into(),
                value: vint(1),
            }],
            original: vec![],
            set: vec![ColumnInput {
                column: "extra".into(),
                value: CellInput::Text("added".into()),
            }],
        }],
    };
    c.driver.preview_changes(&edit).await.unwrap();
    assert_eq!(
        c.driver.apply_changes(&edit).await.unwrap().rows_affected,
        1
    );
    assert_eq!(
        c.scalar(&format!("SELECT extra FROM {table} WHERE id = 1"))
            .await,
        vtext("added")
    );
    assert_eq!(
        c.driver
            .table_structure(&c.tref(&t))
            .await
            .unwrap()
            .columns
            .len(),
        5
    );

    // A primary key is added to a table that had none: it becomes editable at once.
    let loose = c.scratch("ddl_loose");
    let (int, text) = (int_type(c), text_type(c));
    c.side_exec(&format!(
        "CREATE TABLE {} (a {int} NOT NULL, b {text})",
        c.qt(&loose)
    ))
    .await;
    c.side_exec(&format!(
        "INSERT INTO {} VALUES (1, 'x'), (2, 'y')",
        c.qt(&loose)
    ))
    .await;
    let s = c.driver.table_structure(&c.tref(&loose)).await.unwrap();
    assert!(matches!(
        s.editability,
        dbcore::model::Editability::ReadOnly(_)
    ));
    let delete = ChangeSet {
        table: c.tref(&loose),
        changes: vec![RowChange::Delete {
            key: vec![ColumnValue {
                column: "a".into(),
                value: vint(1),
            }],
        }],
    };
    assert!(c.driver.apply_changes(&delete).await.is_err());
    if !c.is_sqlite() {
        c.run(&format!("ALTER TABLE {} ADD PRIMARY KEY (a)", c.qt(&loose)))
            .await;
        assert_eq!(
            c.driver.apply_changes(&delete).await.unwrap().rows_affected,
            1
        );
        assert_eq!(c.side_count(&loose).await, 1);
    }
}
each_engine!(ddl_through_the_editor_refreshes_cached_structures, sql);
