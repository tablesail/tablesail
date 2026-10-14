#!/usr/bin/env bash
# The CI gates, runnable on your machine. .github/workflows/ci.yml calls the same stages,
# so a green run here means a green run there.
#
#   scripts/ci.sh                run every gate
#   scripts/ci.sh lint unit      run only these stages, in the order given
#
# Stages
#   fmt     cargo fmt --check
#   lint    cargo clippy, warnings are errors
#   check   svelte-check and tsc
#   unit    vitest
#   build   vite build (the UI bundle)
#   rust    cargo test --workspace against live Postgres, MySQL, MariaDB and MongoDB
#           (starts them with docker compose; set TABLESAIL_KEEP_DBS=1 to leave them running)
#   e2e     Playwright against the UI with the mock backend (needs Google Chrome)
#   bundle  tauri build, an unsigned installer for this OS (CI does this in build.yml;
#           it is not part of the default run)
#
# Needs `npm ci` to have run once.
set -euo pipefail
cd "$(dirname "$0")/.."

export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"

# The Tauri crate embeds ../dist at compile time and refuses to build without it. Rust-only
# stages only need the folder to exist, so make a placeholder when the UI was not built.
ensure_dist() {
  if [ ! -f dist/index.html ]; then
    mkdir -p dist
    printf '<!doctype html><title>placeholder</title>\n' > dist/index.html
  fi
}

stage_fmt() { cargo fmt --all --check; }

stage_lint() {
  ensure_dist
  cargo clippy --workspace --all-targets -- -D warnings
}

stage_check() { npm run check; }

stage_unit() { npx vitest run; }

stage_build() { npm run build; }

stage_rust() {
  ensure_dist
  local started=0
  if [ "${TABLESAIL_SKIP_DBS:-}" != 1 ]; then
    docker compose up -d --wait --wait-timeout 900 && started=1
  fi
  local status=0
  TABLESAIL_REQUIRE_DBS=1 cargo test --workspace --no-fail-fast || status=$?
  if [ "$status" -ne 0 ] && [ "$started" = 1 ]; then
    docker compose logs --tail 100 || true
  fi
  if [ "$started" = 1 ] && [ "${TABLESAIL_KEEP_DBS:-}" != 1 ] && [ -z "${CI:-}" ]; then
    docker compose stop
  fi
  return "$status"
}

stage_e2e() { npx playwright test; }

stage_bundle() { npx tauri build --config src-tauri/unsigned.conf.json; }

stages=("$@")
[ "${#stages[@]}" -eq 0 ] && stages=(fmt lint check unit build rust e2e)

for s in "${stages[@]}"; do
  if ! declare -F "stage_$s" > /dev/null; then
    echo "unknown stage: $s (fmt lint check unit build rust e2e bundle)" >&2
    exit 2
  fi
done

for s in "${stages[@]}"; do
  printf '\n\033[1m== %s ==\033[0m\n' "$s"
  "stage_$s"
done
printf '\n\033[1mall stages passed: %s\033[0m\n' "${stages[*]}"
