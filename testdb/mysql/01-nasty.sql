-- The "nasty schema" for MySQL 8.4 and MariaDB 11. Both servers load this file;
-- MariaDB-only types live in testdb/mariadb/02-mariadb-only.sql.
-- The docker entrypoint runs it against the `tablesoft` database.

SET NAMES utf8mb4;

-- ---------------------------------------------------------------------------
-- Every type
-- ---------------------------------------------------------------------------
CREATE TABLE all_types (
    id                 BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    c_tinyint          TINYINT,
    c_bool             TINYINT(1),
    c_tinyint_unsigned TINYINT UNSIGNED,
    c_smallint         SMALLINT,
    c_mediumint        MEDIUMINT,
    c_int              INT,
    c_int_unsigned     INT UNSIGNED,
    c_bigint           BIGINT,
    c_bigint_unsigned  BIGINT UNSIGNED,
    c_decimal          DECIMAL(65, 30),
    c_float            FLOAT,
    c_double           DOUBLE,
    c_bit1             BIT(1),
    c_bit64            BIT(64),
    c_char             CHAR(10),
    c_varchar          VARCHAR(255),
    c_binary           BINARY(16),
    c_varbinary        VARBINARY(255),
    c_tinytext         TINYTEXT,
    c_text             TEXT,
    c_mediumtext       MEDIUMTEXT,
    c_longtext         LONGTEXT,
    c_tinyblob         TINYBLOB,
    c_blob             BLOB,
    c_mediumblob       MEDIUMBLOB,
    c_longblob         LONGBLOB,
    c_enum             ENUM('small', 'medium', 'large', 'ünïcødé 🎉'),
    c_set              SET('a', 'b', 'c'),
    c_date             DATE,
    c_datetime         DATETIME(6),
    c_timestamp        TIMESTAMP(6) NULL,
    c_time             TIME(6),
    c_year             YEAR,
    c_json             JSON,
    c_geometry         GEOMETRY,
    c_point            POINT,
    c_linestring       LINESTRING,
    c_polygon          POLYGON,
    c_latin1           VARCHAR(50) CHARACTER SET latin1,
    c_utf8mb4_bin      VARCHAR(50) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- Row 1: ordinary values.
INSERT INTO all_types (
    c_tinyint, c_bool, c_tinyint_unsigned, c_smallint, c_mediumint, c_int, c_int_unsigned,
    c_bigint, c_bigint_unsigned, c_decimal, c_float, c_double, c_bit1, c_bit64, c_char,
    c_varchar, c_binary, c_varbinary, c_tinytext, c_text, c_mediumtext, c_longtext,
    c_tinyblob, c_blob, c_mediumblob, c_longblob, c_enum, c_set, c_date, c_datetime,
    c_timestamp, c_time, c_year, c_json, c_geometry, c_point, c_linestring, c_polygon,
    c_latin1, c_utf8mb4_bin
) VALUES (
    1, 1, 200, 1000, 100000, 42, 4000000000,
    9007199254740993, 18446744073709551615, 12345.678901234567890123456789012345, 1.5, 2.718281828459045,
    b'1', b'1010101010101010101010101010101010101010101010101010101010101010', 'abc',
    'hello', UNHEX('00112233445566778899AABBCCDDEEFF'), X'DEADBEEF', 'tiny', 'text', 'medium', 'long',
    X'01', X'0203', X'040506', X'0708090A', 'medium', 'a,c', '2024-02-29', '2024-02-29 13:37:00.123456',
    '2024-02-29 13:37:00.123456', '838:59:59.000000', 2024,
    '{"a": 1, "nested": {"deep": [true, null, 1.5]}}',
    ST_GeomFromText('POINT(1 1)'), ST_GeomFromText('POINT(1 2)'),
    ST_GeomFromText('LINESTRING(0 0, 1 1, 2 2)'), ST_GeomFromText('POLYGON((0 0, 1 0, 1 1, 0 0))'),
    'café', 'Case Sensitive'
);

-- Row 2: all NULL.
INSERT INTO all_types (c_int) VALUES (NULL);

-- Row 3: extremes and awkward text.
INSERT INTO all_types (
    c_tinyint, c_bool, c_smallint, c_mediumint, c_int, c_bigint, c_bigint_unsigned, c_float,
    c_double, c_bit1, c_char, c_varchar, c_text, c_enum, c_set, c_date, c_datetime, c_time,
    c_year, c_json
) VALUES (
    -128, 0, -32768, -8388608, -2147483648, -9223372036854775808, 0, -3.402823466E+38,
    -1.7976931348623157E+308, b'0', '', '',
    'line one\nline two\ttabbed\r\nwindows line — emoji 🚀 — RTL مرحبا — zero-width​join',
    'ünïcødé 🎉', '', '1000-01-01', '1000-01-01 00:00:00.000000', '-838:59:59.000000',
    1901, '"just a string"'
);

-- Row 4: maximums, tinyint(1) holding a non-boolean, and a 1 MB value.
INSERT INTO all_types (c_tinyint, c_bool, c_int, c_bigint, c_longtext, c_longblob, c_date, c_datetime, c_year, c_json)
VALUES (
    127, 5, 2147483647, 9223372036854775807, REPEAT('x', 1024 * 1024), REPEAT(X'00FF', 4096),
    '9999-12-31', '9999-12-31 23:59:59.999999', 2155, JSON_ARRAY(1, 2, 3)
);

-- ---------------------------------------------------------------------------
-- Zero dates (legacy data that strict mode would reject)
-- ---------------------------------------------------------------------------
CREATE TABLE zero_dates (
    id         INT AUTO_INCREMENT PRIMARY KEY,
    d          DATE,
    dt         DATETIME,
    ts         TIMESTAMP NULL,
    note       VARCHAR(50)
);
SET @old_sql_mode = @@SESSION.sql_mode;
SET SESSION sql_mode = '';
INSERT INTO zero_dates (d, dt, ts, note) VALUES
    ('0000-00-00', '0000-00-00 00:00:00', '0000-00-00 00:00:00', 'all zero'),
    ('2024-00-00', '2024-01-00 00:00:00', NULL, 'zero month/day'),
    ('2024-06-15', '2024-06-15 12:00:00', '2024-06-15 12:00:00', 'normal');
SET SESSION sql_mode = @old_sql_mode;

-- ---------------------------------------------------------------------------
-- Keys: none, composite, unique-only
-- ---------------------------------------------------------------------------
CREATE TABLE no_pk (name VARCHAR(50), value INT);
INSERT INTO no_pk VALUES ('dup', 1), ('dup', 1), ('dup', 1), (NULL, NULL), ('other', 2);

CREATE TABLE composite_pk (
    tenant_id INT,
    item_id   INT,
    label     VARCHAR(50),
    PRIMARY KEY (tenant_id, item_id)
);
INSERT INTO composite_pk VALUES (1, 1, 'a'), (1, 2, 'b'), (2, 1, 'c');

CREATE TABLE unique_only (code VARCHAR(10) NOT NULL UNIQUE, note VARCHAR(50));
INSERT INTO unique_only VALUES ('A', 'first'), ('B', NULL);

CREATE TABLE myisam_table (id INT PRIMARY KEY, note VARCHAR(50)) ENGINE = MyISAM;
INSERT INTO myisam_table VALUES (1, 'no transactions here');

-- ---------------------------------------------------------------------------
-- Names that must be quoted
-- ---------------------------------------------------------------------------
CREATE TABLE `select` (
    `from`  INT PRIMARY KEY,
    `where` VARCHAR(10),
    `order` VARCHAR(10),
    `group` VARCHAR(10),
    `key`   VARCHAR(10),
    `limit` INT
);
INSERT INTO `select` VALUES (1, 'w', 'o', 'g', 'k', 10);

CREATE TABLE `order` (id INT PRIMARY KEY, `desc` VARCHAR(50));
INSERT INTO `order` VALUES (1, 'reserved twice');

CREATE TABLE `weird "quoted" name; DROP TABLE x;--` (
    `column with spaces` INT PRIMARY KEY,
    `column.with.dots`   VARCHAR(10),
    `ñame`               VARCHAR(10),
    `✓ check`            VARCHAR(10),  -- MariaDB identifiers cannot hold 4-byte characters (emoji)
    `123startsWithDigit` VARCHAR(10),
    `back``tick`         VARCHAR(10)
);
INSERT INTO `weird "quoted" name; DROP TABLE x;--` VALUES (1, 'dots', 'ñ', '🗝', 'digit', 'quote');

CREATE TABLE MixedCase (Id INT PRIMARY KEY, camelColumn VARCHAR(10));
INSERT INTO MixedCase VALUES (1, 'camel');

-- ---------------------------------------------------------------------------
-- Relationships, generated columns, views
-- ---------------------------------------------------------------------------
CREATE TABLE customers (
    id    INT AUTO_INCREMENT PRIMARY KEY,
    email VARCHAR(255) NOT NULL UNIQUE,
    name  VARCHAR(100)
);
CREATE TABLE orders (
    id          INT AUTO_INCREMENT PRIMARY KEY,
    customer_id INT NOT NULL,
    total       DECIMAL(12, 2) NOT NULL DEFAULT 0,
    status      ENUM('pending', 'paid', 'shipped') NOT NULL DEFAULT 'pending',
    placed_at   TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT orders_customer_fk FOREIGN KEY (customer_id) REFERENCES customers (id) ON DELETE CASCADE
);
INSERT INTO customers (email, name) VALUES ('a@example.com', 'Alice'), ('b@example.com', 'Bob');
INSERT INTO orders (customer_id, total, status) VALUES (1, 10.00, 'paid'), (1, 20.00, 'pending'), (2, 5.00, 'shipped');

CREATE TABLE categories (
    id        INT AUTO_INCREMENT PRIMARY KEY,
    parent_id INT NULL,
    name      VARCHAR(50),
    FOREIGN KEY (parent_id) REFERENCES categories (id)
);
INSERT INTO categories (parent_id, name) VALUES (NULL, 'root'), (1, 'child'), (2, 'grandchild');

CREATE TABLE computed (
    id    INT AUTO_INCREMENT PRIMARY KEY,
    price DECIMAL(10, 2),
    qty   INT,
    total DECIMAL(12, 2) AS (price * qty) STORED,
    label VARCHAR(20) AS (CONCAT('x', qty)) VIRTUAL
);
INSERT INTO computed (price, qty) VALUES (9.99, 3);

CREATE VIEW customer_totals AS
    SELECT c.id, c.email, SUM(o.total) AS total
    FROM customers c LEFT JOIN orders o ON o.customer_id = c.id
    GROUP BY c.id, c.email;

CREATE TABLE empty_table (id INT PRIMARY KEY, note VARCHAR(50));

-- A procedure returning several result sets.
DELIMITER //
CREATE PROCEDURE multi_results()
BEGIN
    SELECT id, email FROM customers;
    SELECT id, total FROM orders;
    SELECT COUNT(*) AS n FROM orders;
END //
DELIMITER ;

-- A second database for the sidebar.
CREATE DATABASE IF NOT EXISTS tablesoft_other;
CREATE TABLE tablesoft_other.notes (id INT PRIMARY KEY, body TEXT);
INSERT INTO tablesoft_other.notes VALUES (1, 'from another database');
GRANT ALL ON tablesoft_other.* TO 'tablesoft'@'%';

-- ---------------------------------------------------------------------------
-- Wide table: 150 columns, 1000 rows (TEXT: 150 VARCHARs exceed MariaDB's 8 KB row limit)
-- ---------------------------------------------------------------------------
CREATE TABLE digits (d TINYINT UNSIGNED PRIMARY KEY);
INSERT INTO digits VALUES (0), (1), (2), (3), (4), (5), (6), (7), (8), (9);

SET SESSION group_concat_max_len = 1000000;
SELECT CONCAT(
    'CREATE TABLE wide_table (id INT PRIMARY KEY, ',
    GROUP_CONCAT(CONCAT('col_', n, ' TEXT') ORDER BY n SEPARATOR ', '),
    ')'
) INTO @ddl
FROM (SELECT a.d * 100 + b.d * 10 + c.d + 1 AS n FROM digits a, digits b, digits c) cols
WHERE n <= 150;
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SELECT CONCAT(
    'INSERT INTO wide_table SELECT r, ',
    GROUP_CONCAT(CONCAT('CONCAT(''r'', r, ''c', n, ''')') ORDER BY n SEPARATOR ', '),
    ' FROM (SELECT a.d * 100 + b.d * 10 + c.d + 1 AS r FROM digits a, digits b, digits c) rows_'
) INTO @dml
FROM (SELECT a.d * 100 + b.d * 10 + c.d + 1 AS n FROM digits a, digits b, digits c) cols
WHERE n <= 150;
PREPARE stmt FROM @dml;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

-- ---------------------------------------------------------------------------
-- 1M rows
-- ---------------------------------------------------------------------------
CREATE TABLE big_table (
    id         BIGINT PRIMARY KEY,
    uuid       CHAR(36) NOT NULL,
    name       VARCHAR(50) NOT NULL,
    amount     DECIMAL(12, 2),
    is_active  TINYINT(1) NOT NULL,
    created_at DATETIME NOT NULL,
    payload    JSON
);

INSERT INTO big_table
SELECT
    n,
    CONCAT_WS('-', SUBSTR(h, 1, 8), SUBSTR(h, 9, 4), SUBSTR(h, 13, 4), SUBSTR(h, 17, 4), SUBSTR(h, 21, 12)),
    CONCAT('row ', n),
    IF(n % 10 = 0, NULL, (n % 100000) / 100),
    n % 2 = 0,
    TIMESTAMPADD(MINUTE, n, '2020-01-01 00:00:00'),
    IF(n % 3 = 0, JSON_OBJECT('n', n, 'tags', JSON_ARRAY('a', 'b')), NULL)
FROM (
    SELECT n, MD5(n) AS h
    FROM (
        SELECT a.d + b.d * 10 + c.d * 100 + d.d * 1000 + e.d * 10000 + f.d * 100000 + 1 AS n
        FROM digits a, digits b, digits c, digits d, digits e, digits f
    ) seq
) src;

CREATE INDEX big_table_created_at_idx ON big_table (created_at);
ANALYZE TABLE big_table;
