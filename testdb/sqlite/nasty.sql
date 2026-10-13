-- The "nasty schema" for SQLite. Build a file from it with scripts/make-sqlite-db.sh,
-- or let crates/dbcore/tests load it into memory.

PRAGMA foreign_keys = ON;

-- Declared types are only hints in SQLite: every column below can hold anything.
CREATE TABLE all_types (
    id             INTEGER PRIMARY KEY,
    c_integer      INTEGER,
    c_real         REAL,
    c_text         TEXT,
    c_blob         BLOB,
    c_numeric      NUMERIC,
    c_no_type,
    c_varchar      VARCHAR(10),
    c_boolean      BOOLEAN,
    c_datetime     DATETIME,
    c_date         DATE,
    c_json         JSON,
    c_decimal      DECIMAL(10, 2),
    c_bigint       BIGINT,
    c_unknown_type FANCY_CUSTOM_TYPE
);

INSERT INTO all_types VALUES
    (1, 42, 1.5, 'hello', X'DEADBEEF', 12.5, 'untyped', 'short', 1,
     '2024-02-29 13:37:00', '2024-02-29', '{"a": 1, "b": [1, 2]}', 19.99, 9007199254740993, 'x'),
    (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL),
    -- Wrong types in every column: SQLite keeps them.
    (3, 'not a number', 'text in real', 123, 'text in blob', X'00', 3.14, 'way longer than ten characters',
     'yes', 1709213820, 'not a date', 'not json', 'abc', -9223372036854775808, NULL),
    (4, 9223372036854775807, 1e308, 'line one' || char(10) || 'line two — emoji 🚀 — RTL مرحبا',
     zeroblob(65536), '1e400', X'', '', 0, 'infinity', '0000-00-00', '"string"', 0.1, 0, '');

CREATE TABLE strict_table (id INTEGER PRIMARY KEY, n INTEGER NOT NULL, label TEXT, data ANY) STRICT;
INSERT INTO strict_table VALUES (1, 10, 'strict', X'01'), (2, 20, NULL, 'any type ok');

-- Tables without a usable primary key.
CREATE TABLE no_pk (name TEXT, value INTEGER);
INSERT INTO no_pk VALUES ('dup', 1), ('dup', 1), ('dup', 1), (NULL, NULL), ('other', 2);

CREATE TABLE without_rowid (code TEXT PRIMARY KEY, note TEXT) WITHOUT ROWID;
INSERT INTO without_rowid VALUES ('A', 'first'), ('B', NULL);

CREATE TABLE composite_pk (tenant_id INTEGER, item_id INTEGER, label TEXT, PRIMARY KEY (tenant_id, item_id));
INSERT INTO composite_pk VALUES (1, 1, 'a'), (1, 2, 'b'), (2, 1, 'c');

-- Names that must be quoted.
CREATE TABLE "select" ("from" INTEGER PRIMARY KEY, "where" TEXT, "order" TEXT, "group" TEXT, "limit" INTEGER);
INSERT INTO "select" VALUES (1, 'w', 'o', 'g', 10);

CREATE TABLE "weird ""quoted"" name; DROP TABLE x;--" (
    "column with spaces" INTEGER PRIMARY KEY,
    "column.with.dots"   TEXT,
    "ñame"               TEXT,
    "🔑"                  TEXT,
    "123startsWithDigit" TEXT,
    [bracketed]          TEXT,
    `backticked`         TEXT
);
INSERT INTO "weird ""quoted"" name; DROP TABLE x;--" VALUES (1, 'dots', 'ñ', '🗝', 'digit', 'br', 'bt');

-- Relationships, generated columns, views.
CREATE TABLE customers (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE COLLATE NOCASE, name TEXT);
CREATE TABLE orders (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    customer_id INTEGER NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
    total       REAL NOT NULL DEFAULT 0,
    placed_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX orders_customer_idx ON orders (customer_id);
INSERT INTO customers (email, name) VALUES ('a@example.com', 'Alice'), ('b@example.com', 'Bob');
INSERT INTO orders (customer_id, total) VALUES (1, 10.0), (1, 20.0), (2, 5.0);

CREATE TABLE computed (
    id    INTEGER PRIMARY KEY,
    price REAL,
    qty   INTEGER,
    total REAL GENERATED ALWAYS AS (price * qty) STORED,
    label TEXT GENERATED ALWAYS AS ('x' || qty) VIRTUAL
);
INSERT INTO computed (price, qty) VALUES (9.99, 3);

CREATE VIEW customer_totals AS
    SELECT c.id, c.email, SUM(o.total) AS total
    FROM customers c LEFT JOIN orders o ON o.customer_id = c.id
    GROUP BY c.id, c.email;

CREATE TABLE empty_table (id INTEGER PRIMARY KEY, note TEXT);

-- Virtual table with shadow tables a sidebar should hide.
CREATE VIRTUAL TABLE docs_fts USING fts5(title, body);
INSERT INTO docs_fts VALUES ('hello', 'full text search body');

-- 1M rows.
CREATE TABLE big_table (
    id         INTEGER PRIMARY KEY,
    uuid       TEXT NOT NULL,
    name       TEXT NOT NULL,
    amount     REAL,
    is_active  INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    payload    TEXT
);

WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 1000000)
INSERT INTO big_table
SELECT
    n,
    lower(hex(randomblob(16))),
    'row ' || n,
    CASE WHEN n % 10 = 0 THEN NULL ELSE (n % 100000) / 100.0 END,
    n % 2 = 0,
    datetime('2020-01-01 00:00:00', '+' || n || ' minutes'),
    CASE WHEN n % 3 = 0 THEN json_object('n', n, 'tags', json_array('a', 'b')) END
FROM seq;

CREATE INDEX big_table_created_at_idx ON big_table (created_at);
ANALYZE;
