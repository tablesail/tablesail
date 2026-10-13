# TableSoft

A fast, open-source desktop database client: Svelte UI → Tauri → Rust core,
shipping as one binary for macOS, Windows and Linux. Laravel-aware features are
the differentiator. See [PLAN.md](PLAN.md) for the roadmap and hard targets.

## Layout

| Path | What |
|---|---|
| `src/` | Svelte + TypeScript UI |
| `src-tauri/` | Tauri shell (the `tablesoft` crate); thin, calls into `dbcore` |
| `crates/dbcore/` | Database core: drivers, queries, introspection. No Tauri dependency |
| `testdb/` | "Nasty schema" seeds for every engine |
| `docker-compose.yml` | Postgres 17, MySQL 8.4, MariaDB 11.8, MongoDB 8.2 test servers |
| `scripts/` | Helper scripts |

## Prerequisites

- Rust (stable) via [rustup](https://rustup.rs)
- Node 24 and npm
- Docker (for the test databases)
- Platform prerequisites for Tauri: <https://v2.tauri.app/start/prerequisites/>

## Run the app

```sh
npm install
npm run tauri dev
```

The first run compiles all Rust dependencies and takes a few minutes; later runs
are incremental. UI changes hot-reload without rebuilding Rust.

## Test databases

```sh
docker compose up -d --wait   # start and wait until every seed has finished (first run: a few minutes)
docker compose down -v        # stop and delete the data; the next `up` re-seeds
scripts/make-sqlite-db.sh     # build data/nasty.db (SQLite, 1M rows)
```

| Engine | URL |
|---|---|
| PostgreSQL | `postgres://tablesoft:tablesoft@127.0.0.1:25432/tablesoft` |
| MySQL | `mysql://tablesoft:tablesoft@127.0.0.1:23306/tablesoft` |
| MariaDB | `mysql://tablesoft:tablesoft@127.0.0.1:23307/tablesoft` |
| MongoDB | `mongodb://127.0.0.1:27117/tablesoft` |

Ports are non-default so they never collide with a locally installed server.
Every engine is seeded with the same "nasty schema": every column type, tables
without primary keys, composite keys, reserved-word and unicode names, views,
a 150-column table and a 1M-row `big_table` (`big_collection` in Mongo).

## Tests

```sh
cargo test -p dbcore
```

SQLite always runs (in memory). Server engines are skipped with a message when
their container is not running; set `TABLESOFT_REQUIRE_DBS=1` to make that a
failure instead, as CI does.
