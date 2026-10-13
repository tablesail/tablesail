-- MariaDB-only additions to the nasty schema (runs after testdb/mysql/01-nasty.sql).

CREATE TABLE mariadb_types (
    id      INT AUTO_INCREMENT PRIMARY KEY,
    c_uuid  UUID,
    c_inet4 INET4,
    c_inet6 INET6,
    c_json  JSON                -- an alias for LONGTEXT with a JSON_VALID check
);
INSERT INTO mariadb_types (c_uuid, c_inet4, c_inet6, c_json) VALUES
    ('a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '192.168.0.1', '2001:db8::1', '{"a": 1}'),
    (NULL, NULL, NULL, NULL);

-- Sequences show up as tables in information_schema.
CREATE SEQUENCE invoice_seq START WITH 1000 INCREMENT BY 10;

-- System-versioned table: hidden row_start/row_end columns.
CREATE TABLE versioned (id INT PRIMARY KEY, price DECIMAL(10, 2)) WITH SYSTEM VERSIONING;
INSERT INTO versioned VALUES (1, 10.00);
UPDATE versioned SET price = 12.00 WHERE id = 1;
