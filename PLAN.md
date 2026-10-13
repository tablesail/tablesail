# TablePlus Alternative — Build Plan

Oct 13, 2026 · @Mr Hawk

## Locked decisions

A desktop database client built as Svelte UI → Tauri → Rust core, shipping as one binary for macOS, Windows and Linux. Speed is a hard requirement; the Laravel-aware features are the differentiator.

| Area | Decision |
|---|---|
| UI | Svelte + TypeScript, virtualized grid, CodeMirror 6 editor |
| Shell | Tauri (OS web view, no bundled browser) |
| Core | Rust: sqlx (MySQL/MariaDB, PostgreSQL, SQLite), mongodb, russh, keyring, tokio |
| Engines in v1 | MySQL/MariaDB, PostgreSQL, SQLite, MongoDB |
| Differentiator | Free and open source, faster than TablePlus on the hard targets below. Laravel-aware features deferred to after v1.0 (decided Oct 13, 2026) |
| Licence | Open source |
| Who builds | Claude writes all code; Mr Hawk installs the toolchain, runs, reviews and packages |

## Phase 0 — Setup (1 session)

The repo, the test databases and CI exist before any feature code.

- Git repo; Tauri + Svelte + TypeScript scaffold; Rust workspace with a separate dbcore crate
- docker-compose.yml with Postgres, MySQL, MariaDB and Mongo, seeded with a "nasty schema": every type, PK-less tables, reserved-word names, a 1M-row table
- CI runs cargo test against all four engines on every push
- [ ] Mr Hawk: install Rust, Node and the Tauri prerequisites; confirm `npm run tauri dev` opens a blank window

## Phase 1 — Core + SQLite (2 sessions)

Done when a 1M-row SQLite table shows its first row in under 100 ms and scrolls at 60 fps.

- Driver trait, typed Value enum, normalized column types
- SQLite adapter, schema introspection, streaming query cursor with cancel
- UI: connection list, sidebar (databases → tables), read-only virtualized grid, open a .db file

## Phase 2 — SQL editor (1–2 sessions)

- CodeMirror 6 with schema-aware autocomplete
- Run the statement under the cursor; multi-statement scripts; result tabs; a cancel button that works
- Statement splitter that respects strings and comments

## Phase 3 — PostgreSQL (2 sessions)

- sqlx Postgres adapter; schemas beyond public, views, enums, arrays, JSONB, timestamptz, bigint kept as string
- SSL modes and certificate handling
- Schema cache on disk so the second launch is instant

## Phase 4 — Editing (2 sessions)

Edits accumulate as pending changes, preview as SQL, and commit in one transaction.

- ChangeSet of inserts/updates/deletes keyed on the primary key, original values in the WHERE clause
- Preview SQL → Commit → full rollback on any error, showing the server's message
- Draft persistence on crash; editing blocked on views and PK-less tables with a stated reason
- Typed cell editors: JSON viewer, date picker, NULL toggle, binary preview

## Phase 5 — MySQL/MariaDB + connections (2 sessions)

- sqlx MySQL adapter; auth plugins, tinyint(1), zero dates, multiple result sets
- SSH tunnels: key, passphrase, agent, jump host, auto-reconnect; passwords in the OS keychain
- Environment tags (local / staging / prod), colored window chrome, confirmation on destructive statements against prod

## Phase 6 — Export/import + polish (1–2 sessions) — v0.1 public beta

Milestone: usable daily on SQL databases.

- Streaming CSV/JSON/SQL export straight from the cursor to disk; CSV import
- Keyboard shortcuts, tabs, dark mode, high-DPI; filters and sorting pushed down to SQL

## Phase 7 — MongoDB (2 sessions) — v1.0

- Official Rust driver behind the same Driver trait: collections as tables, sampled schema
- Document viewer and filter editor
- BSON types kept typed end to end: ObjectId, Decimal128, Int64, Date, Binary

## Phase 8 — Laravel features — deferred until after v1.0

The features TablePlus will never build, aimed at the Laravel community.

- Point at a project folder → connections created from .env
- Migration status shown beside each table
- Generate model, factory or migration from a table

## Phase 9 — Release

- Tauri bundler: .dmg, .msi, .AppImage
- Signed auto-updater fed by a static JSON manifest
- GitHub Releases, landing page, open-source licence

## Hard performance targets

Benchmarked against TablePlus at every milestone, not at the end.

| Measure | Target |
|---|---|
| Cold launch | < 1 s |
| First row, local database | < 100 ms |
| First row, remote database | network round trip + 50 ms |
| Scrolling a 1M-row result | 60 fps |
| UI responsiveness | never blocks; every long operation is cancellable |
| Memory | bounded regardless of result size |
| Export | disk speed, streamed from the cursor |

## Risks to watch

| Risk | Mitigation |
|---|---|
| Rust compile times slow iteration | Keep dbcore small; the UI iterates without rebuilding Rust |
| Web-view quirks per OS (WebKitGTK on Linux is the weakest) | Test on all three platforms from Phase 1, not at release |
| Mongo's schemaless shape fights the grid model | Scheduled last, behind a stable trait and grid |
| Query cancel is not server-side in sqlx (dropping a stream stops reading, the query keeps running); sqlx also lacks MariaDB's ed25519 auth | `Driver` trait gets an explicit `cancel()` from Phase 1. Per engine: Postgres cancel request (tokio-postgres `cancel_token`), MySQL/MariaDB `KILL QUERY` on a side connection, SQLite `sqlite3_interrupt`. Driver crate per engine stays open until Phase 1 is designed |

## Timeline and working rules

v0.1 public beta after Phase 6, v1.0 after Phase 7. On Oct 13, 2026 Mr Hawk chose to write all phases' code first (one workflow of parallel agents against a fixed contract) and test afterwards; rules 1, 2 and 4 apply from the debugging pass onwards.

1. One milestone per session; every session ends with a working app, never a half-wired state
2. The test suite against all four engines passes before a milestone is called done
3. Every change lands as a commit Mr Hawk can diff and review
4. Hard targets are measured at each milestone against TablePlus on the same machine
