#!/usr/bin/env sh
# Build the SQLite nasty-schema database used for manual testing and benchmarks.
#   scripts/make-sqlite-db.sh [output path]   (default: data/nasty.db)
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-"$root/data/nasty.db"}

mkdir -p "$(dirname "$out")"
rm -f "$out"
sqlite3 "$out" < "$root/testdb/sqlite/nasty.sql"
echo "Wrote $out ($(du -h "$out" | cut -f1))"
