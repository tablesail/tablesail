-- The "nasty schema" for PostgreSQL: every type, awkward names, PK-less tables,
-- and a 1M-row table. Anything a database client can get wrong should be here.

CREATE EXTENSION IF NOT EXISTS hstore;
CREATE EXTENSION IF NOT EXISTS citext;

-- ---------------------------------------------------------------------------
-- Custom types
-- ---------------------------------------------------------------------------
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy', 'ünïcødé 🎉');
CREATE TYPE address AS (street text, city text, zip varchar(10));
CREATE DOMAIN positive_int AS integer CHECK (VALUE > 0);

-- ---------------------------------------------------------------------------
-- Every built-in type worth showing in a grid
-- ---------------------------------------------------------------------------
CREATE TABLE all_types (
    id              bigserial PRIMARY KEY,
    c_smallint      smallint,
    c_integer       integer,
    c_bigint        bigint,
    c_numeric       numeric(38, 10),
    c_numeric_free  numeric,
    c_real          real,
    c_double        double precision,
    c_money         money,
    c_boolean       boolean,
    c_char          char(10),
    c_varchar       varchar(255),
    c_text          text,
    c_citext        citext,
    c_bytea         bytea,
    c_date          date,
    c_time          time,
    c_timetz        timetz,
    c_timestamp     timestamp(6),
    c_timestamptz   timestamptz(6),
    c_interval      interval,
    c_uuid          uuid,
    c_json          json,
    c_jsonb         jsonb,
    c_xml           xml,
    c_inet          inet,
    c_cidr          cidr,
    c_macaddr       macaddr,
    c_macaddr8      macaddr8,
    c_point         point,
    c_line          line,
    c_lseg          lseg,
    c_box           box,
    c_path          path,
    c_polygon       polygon,
    c_circle        circle,
    c_bit           bit(8),
    c_varbit        bit varying(64),
    c_tsvector      tsvector,
    c_tsquery       tsquery,
    c_int4range     int4range,
    c_tstzrange     tstzrange,
    c_int_array     integer[],
    c_text_array    text[],
    c_2d_array      integer[][],
    c_jsonb_array   jsonb[],
    c_mood          mood,
    c_mood_array    mood[],
    c_address       address,
    c_domain        positive_int,
    c_hstore        hstore,
    c_oid           oid,
    c_pg_lsn        pg_lsn
);

-- Row 1: ordinary values.
INSERT INTO all_types (
    c_smallint, c_integer, c_bigint, c_numeric, c_numeric_free, c_real, c_double, c_money,
    c_boolean, c_char, c_varchar, c_text, c_citext, c_bytea, c_date, c_time, c_timetz,
    c_timestamp, c_timestamptz, c_interval, c_uuid, c_json, c_jsonb, c_xml, c_inet, c_cidr,
    c_macaddr, c_macaddr8, c_point, c_line, c_lseg, c_box, c_path, c_polygon, c_circle,
    c_bit, c_varbit, c_tsvector, c_tsquery, c_int4range, c_tstzrange, c_int_array,
    c_text_array, c_2d_array, c_jsonb_array, c_mood, c_mood_array, c_address, c_domain,
    c_hstore, c_oid, c_pg_lsn
) VALUES (
    1, 42, 9007199254740993, 12345.6789012345, 3.14159265358979323846264338327950288419716939937510,
    1.5, 2.718281828459045, 19.99,
    true, 'abc', 'hello', 'plain text', 'CaseInsensitive', '\xdeadbeef', '2024-02-29', '13:37:00',
    '13:37:00+05:30', '2024-02-29 13:37:00.123456', '2024-02-29 13:37:00.123456+00',
    '1 year 2 mons 3 days 04:05:06.789', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11',
    '{"a": 1, "b": [1, 2, 3]}', '{"nested": {"deep": {"deeper": [true, null, 1.5]}}}',
    '<root><child attr="x">text</child></root>', '192.168.0.1', '10.0.0.0/8',
    '08:00:2b:01:02:03', '08:00:2b:01:02:03:04:05', '(1,2)', '{1,-1,0}', '[(0,0),(1,1)]',
    '((0,0),(1,1))', '[(0,0),(1,1),(2,0)]', '((0,0),(1,1),(1,0))', '<(0,0),5>',
    B'10101010', B'101', 'the quick brown fox', 'quick & fox', '[1,10)',
    '[2024-01-01 00:00:00+00, 2024-12-31 23:59:59+00)', '{1,2,3}', '{"a","b,c","d\"e",NULL}',
    '{{1,2},{3,4}}', ARRAY['{"x":1}'::jsonb, '[]'::jsonb], 'happy', '{sad,ok}',
    ROW('1 Main St', 'Springfield', '12345'), 7, 'key=>value, "with space"=>"x"', 1259, '16/B374D848'
);

-- Row 2: all NULL.
INSERT INTO all_types (c_integer) VALUES (NULL);

-- Row 3: extremes and things that break naive clients.
INSERT INTO all_types (
    c_smallint, c_integer, c_bigint, c_numeric_free, c_real, c_double, c_money, c_boolean,
    c_char, c_varchar, c_text, c_bytea, c_date, c_timestamp, c_timestamptz, c_interval,
    c_json, c_jsonb, c_int_array, c_text_array, c_mood, c_tsvector
) VALUES (
    -32768, -2147483648, -9223372036854775808, 'NaN', '-Infinity', 'Infinity',
    -92233720368547758.08, false,
    '', '', E'line one\nline two\ttabbed\r\nwindows line — emoji 🚀 — RTL مرحبا — zero-width​join',
    '\x', '0001-01-01', '-infinity', 'infinity', '-178000000 years',
    'null', '"just a string"', '{}', '{}', 'ünïcødé 🎉', ''
);

-- Row 4: maximums and a 1 MB text value.
INSERT INTO all_types (c_smallint, c_integer, c_bigint, c_numeric_free, c_text, c_bytea, c_date, c_jsonb)
VALUES (
    32767, 2147483647, 9223372036854775807,
    99999999999999999999999999999999999999.99999999999999999999,
    repeat('x', 1024 * 1024),
    decode(repeat('00ff', 4096), 'hex'),
    '5874897-12-31',
    (SELECT jsonb_agg(g) FROM generate_series(1, 5000) g)
);

-- ---------------------------------------------------------------------------
-- Keys: none, composite, unique-only
-- ---------------------------------------------------------------------------
CREATE TABLE no_pk (name text, value integer);
INSERT INTO no_pk VALUES ('dup', 1), ('dup', 1), ('dup', 1), (NULL, NULL), ('other', 2);

CREATE TABLE composite_pk (
    tenant_id integer,
    item_id   integer,
    label     text,
    PRIMARY KEY (tenant_id, item_id)
);
INSERT INTO composite_pk VALUES (1, 1, 'a'), (1, 2, 'b'), (2, 1, 'c');

CREATE TABLE unique_only (code text NOT NULL UNIQUE, note text);
INSERT INTO unique_only VALUES ('A', 'first'), ('B', NULL);

-- ---------------------------------------------------------------------------
-- Names that must be quoted
-- ---------------------------------------------------------------------------
CREATE TABLE "select" (
    "from"   integer PRIMARY KEY,
    "where"  text,
    "order"  text,
    "group"  text,
    "user"   text,
    "limit"  integer
);
INSERT INTO "select" VALUES (1, 'w', 'o', 'g', 'u', 10);

CREATE TABLE "user" (id serial PRIMARY KEY, "name" text, "password" text);
INSERT INTO "user" ("name", "password") VALUES ('admin', 'hunter2');

CREATE TABLE "MixedCase" ("Id" integer PRIMARY KEY, "camelColumn" text, "UPPER" text);
INSERT INTO "MixedCase" VALUES (1, 'camel', 'UP');

CREATE TABLE "weird ""quoted"" name; DROP TABLE x;--" (
    "column with spaces"   integer PRIMARY KEY,
    "column.with.dots"     text,
    "ñame"                 text,
    "🔑"                    text,
    "123startsWithDigit"   text,
    """ "                  text
);
INSERT INTO "weird ""quoted"" name; DROP TABLE x;--" VALUES (1, 'dots', 'ñ', '🗝', 'digit', 'quote');

-- ---------------------------------------------------------------------------
-- Schemas beyond public
-- ---------------------------------------------------------------------------
CREATE SCHEMA billing;
CREATE TABLE billing.invoices (id serial PRIMARY KEY, amount numeric(12, 2), issued_on date);
INSERT INTO billing.invoices (amount, issued_on) VALUES (100.00, '2024-01-01'), (250.50, '2024-02-01');

CREATE SCHEMA "Mixed Case Schema";
CREATE TABLE "Mixed Case Schema"."select" (id integer PRIMARY KEY);
-- Same table name as billing.invoices, in another schema.
CREATE TABLE public.invoices (id serial PRIMARY KEY, note text);

CREATE SCHEMA empty_schema;

-- ---------------------------------------------------------------------------
-- Relationships
-- ---------------------------------------------------------------------------
CREATE TABLE customers (id serial PRIMARY KEY, email citext UNIQUE NOT NULL, name text);
CREATE TABLE orders (
    id          serial PRIMARY KEY,
    customer_id integer NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
    total       numeric(12, 2) NOT NULL DEFAULT 0,
    status      mood,
    placed_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX orders_customer_idx ON orders (customer_id);
INSERT INTO customers (email, name) VALUES ('a@example.com', 'Alice'), ('b@example.com', 'Bob');
INSERT INTO orders (customer_id, total, status) VALUES (1, 10.00, 'happy'), (1, 20.00, 'ok'), (2, 5.00, 'sad');

-- Self-reference.
CREATE TABLE categories (id serial PRIMARY KEY, parent_id integer REFERENCES categories (id), name text);
INSERT INTO categories (parent_id, name) VALUES (NULL, 'root'), (1, 'child'), (2, 'grandchild');

-- Generated and identity columns.
CREATE TABLE computed (
    id        integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    price     numeric(10, 2),
    qty       integer,
    total     numeric(12, 2) GENERATED ALWAYS AS (price * qty) STORED
);
INSERT INTO computed (price, qty) VALUES (9.99, 3);

-- ---------------------------------------------------------------------------
-- Views, partitions, inheritance
-- ---------------------------------------------------------------------------
CREATE VIEW customer_totals AS
    SELECT c.id, c.email, sum(o.total) AS total
    FROM customers c LEFT JOIN orders o ON o.customer_id = c.id
    GROUP BY c.id, c.email;

CREATE MATERIALIZED VIEW mood_counts AS
    SELECT status, count(*) AS n FROM orders GROUP BY status;

CREATE TABLE events (id bigint, happened_on date NOT NULL, payload jsonb) PARTITION BY RANGE (happened_on);
CREATE TABLE events_2024 PARTITION OF events FOR VALUES FROM ('2024-01-01') TO ('2025-01-01');
CREATE TABLE events_2025 PARTITION OF events FOR VALUES FROM ('2025-01-01') TO ('2026-01-01');
INSERT INTO events VALUES (1, '2024-06-01', '{}'), (2, '2025-06-01', '{}');

CREATE TABLE empty_table (id integer PRIMARY KEY, note text);

-- ---------------------------------------------------------------------------
-- Wide table: 150 columns
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    cols text;
BEGIN
    SELECT string_agg(format('col_%s text', i), ', ') INTO cols FROM generate_series(1, 150) i;
    EXECUTE format('CREATE TABLE wide_table (id serial PRIMARY KEY, %s)', cols);
    EXECUTE format(
        'INSERT INTO wide_table (%s) SELECT %s FROM generate_series(1, 1000) r',
        (SELECT string_agg(format('col_%s', i), ', ') FROM generate_series(1, 150) i),
        (SELECT string_agg(format('''r'' || r || ''c%s''', i), ', ') FROM generate_series(1, 150) i)
    );
END
$$;

-- ---------------------------------------------------------------------------
-- 1M rows
-- ---------------------------------------------------------------------------
CREATE TABLE big_table (
    id         bigint PRIMARY KEY,
    uuid       uuid NOT NULL,
    name       text NOT NULL,
    amount     numeric(12, 2),
    is_active  boolean NOT NULL,
    created_at timestamptz NOT NULL,
    payload    jsonb
);

INSERT INTO big_table
SELECT
    g,
    md5(g::text)::uuid,
    'row ' || g,
    CASE WHEN g % 10 = 0 THEN NULL ELSE (g % 100000) / 100.0 END,
    g % 2 = 0,
    timestamptz '2020-01-01 00:00:00+00' + g * interval '1 minute',
    CASE WHEN g % 3 = 0 THEN jsonb_build_object('n', g, 'tags', jsonb_build_array('a', 'b')) END
FROM generate_series(1, 1000000) g;

CREATE INDEX big_table_created_at_idx ON big_table (created_at);
ANALYZE;
