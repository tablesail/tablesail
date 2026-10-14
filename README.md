# TableSail

A fast, free, open-source desktop database client for **PostgreSQL, MySQL, MariaDB,
SQLite and MongoDB**. One small native app for macOS, Windows and Linux, built with
Svelte, Tauri and Rust. MIT licensed, no licence nags.

> **Status: early (v0.1).** Every engine is tested against live servers and the whole UI
> has a browser test suite, but the packaged app has so far only been exercised on macOS.
> SSH tunnels, TLS modes and read-only mode are implemented and not yet tested against real
> servers. The speed figures in [PLAN.md](PLAN.md) are design targets, not benchmark results.

## Features

**Browse**
- Virtualized data grid: only the visible rows are rendered, results stream from a cursor,
  and every long operation can be cancelled
- Row inspector sidebar with pretty-printed JSON
- Filters and sorting pushed down to SQL, structure view for columns, keys and indexes

**Query**
- SQL editor with highlighting and schema-aware autocomplete
- Run the statement under the cursor or the whole script; results in tabs
- Open, save and import `.sql` files and SQL dumps
- Manual transactions with Begin, Commit and Rollback, and a prompt before you lose one
- Query console that logs everything TableSail sends to the server
- Optional AI assistant that writes SQL for the open connection (bring your own Anthropic
  API key, kept in the OS keychain)

**Edit safely**
- Edits collect as pending changes, preview as SQL, and commit in one transaction
- Copy rows as INSERT, JSON or Markdown, and paste into the grid
- Drop, truncate and rename tables from the sidebar, with a preview of the SQL; dropping
  asks you to type the table name

**Connect**
- Paste a connection URL for any engine
- SSH tunnels, with key, passphrase, agent and jump-host support
- Passwords live in the OS keychain
- Environment tags (local, staging, production) colour each connection so production never
  looks like staging

**Move data**
- Stream large results to CSV, JSON, JSON Lines or SQL INSERTs without holding them in memory
- Import CSV

Press <kbd>⌘K</kbd> (<kbd>Ctrl K</kbd>) for the command palette and <kbd>⌘/</kbd>
(<kbd>Ctrl /</kbd>) for every shortcut.

## Install

Download the installer for your platform from
[Releases](https://github.com/tablesail/tablesail/releases):

| OS | File |
|---|---|
| macOS | `.dmg` (Apple Silicon and Intel) |
| Windows | `.exe` installer |
| Linux | `.deb` or `.AppImage` |

The builds are not code-signed yet, so the first launch needs one extra step:

- **macOS:** drag TableSail to Applications, then run `xattr -cr /Applications/TableSail.app`
  (or right-click the app and choose Open)
- **Windows:** SmartScreen shows "Windows protected your PC": choose More info, then Run anyway
- **Linux:** `chmod +x TableSail*.AppImage` before running it

## Build from source

You need [Rust](https://rustup.rs) (stable), Node 24 and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS. On Debian or
Ubuntu the system packages are listed in
[`.github/actions/linux-deps`](.github/actions/linux-deps/action.yml).

```sh
npm install
npm run tauri dev      # run the app; UI changes hot-reload without rebuilding Rust
npm run ci -- bundle   # an unsigned installer for your OS (normally CI builds these)
```

To open just the UI in a browser against built-in sample data, with no Rust at all:
`npm run dev`, then <http://localhost:5173>.

## Tests

Everything CI runs is one command, and each stage can be run alone:

```sh
npm run ci                 # every gate below
npm run ci -- lint unit    # only these stages
```

| Stage | Runs |
|---|---|
| `fmt` | `cargo fmt --check` |
| `lint` | `cargo clippy`, warnings are errors |
| `check` | `svelte-check` and `tsc` |
| `unit` | Vitest |
| `build` | the Vite production build |
| `rust` | `cargo test --workspace` against live Postgres, MySQL, MariaDB and MongoDB |
| `e2e` | Playwright, driving the UI in Google Chrome against the mock backend |

`rust` starts the test databases with Docker (below). SQLite always runs in memory, and a
server engine whose container is down is skipped with a message unless
`TABLESAIL_REQUIRE_DBS=1` is set, which `rust` does.

### Test databases

```sh
docker compose up -d --wait   # start and wait until every seed has finished (first run: a few minutes)
docker compose down -v        # stop and delete the data; the next `up` re-seeds
scripts/make-sqlite-db.sh     # build data/nasty.db (SQLite, 1M rows)
```

| Engine | URL |
|---|---|
| PostgreSQL | `postgres://tablesail:tablesail@127.0.0.1:25432/tablesail` |
| MySQL | `mysql://tablesail:tablesail@127.0.0.1:23306/tablesail` |
| MariaDB | `mysql://tablesail:tablesail@127.0.0.1:23307/tablesail` |
| MongoDB | `mongodb://127.0.0.1:27117/tablesail` |

The ports are non-default so they never collide with a local server. Every engine is seeded
with the same "nasty schema": every column type, tables without primary keys, composite
keys, reserved-word and unicode names, views, a 150-column table and a 1M-row `big_table`
(`big_collection` in Mongo).

## CI and installers

| Workflow | When | What |
|---|---|---|
| [`ci.yml`](.github/workflows/ci.yml) | push to `main`, pull requests | the gates above, as parallel jobs |
| [`build.yml`](.github/workflows/build.yml) | run by hand | unsigned installers for macOS (Apple Silicon and Intel), Windows and Linux, attached to the run as artifacts |
| [`release.yml`](.github/workflows/release.yml) | a `v*` tag | the same installers on a draft GitHub Release, plus the auto-update manifest when the signing key is set |

To get installers without releasing: open the repository's Actions tab, choose **Build
installers**, and run it. Or from a terminal:

```sh
gh workflow run build.yml
gh run watch
gh run download --dir installers     # when the run is done
```

Installers are built on GitHub's runners, so you do not need any of the toolchain locally.
Releases, signing keys and the auto-updater are covered in
[docs/RELEASING.md](docs/RELEASING.md).

## Layout

| Path | What |
|---|---|
| `src/` | Svelte + TypeScript UI |
| `src-tauri/` | Tauri shell (the `tablesail` crate): thin commands that call into `dbcore` |
| `crates/dbcore/` | Database core: drivers, queries, introspection, editing, export. No Tauri dependency |
| `e2e/` | Playwright tests |
| `testdb/` | "Nasty schema" seeds for every engine |
| `scripts/` | `ci.sh` (the CI gates) and helpers |
| `site/` | The landing page, plain static HTML |
| `docs/` | Release notes for maintainers |

[PLAN.md](PLAN.md) has the roadmap and the performance targets.

## Licence

[MIT](LICENSE)
