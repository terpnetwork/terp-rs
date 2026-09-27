#!/usr/bin/env bash
# Shared helpers for local CI exercise (host just + nektos act).
# shellcheck disable=SC2034

# Resolve paths: this file lives at <crate>/scripts/ci/lib.sh
_ACT_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_ROOT="$(cd "${_ACT_LIB_DIR}/../.." && pwd)"
# monorepo: .../terp-core  (parent of crates/)
MONOREPO_ROOT="$(cd "${CRATE_ROOT}/../.." && pwd)"

RUNS_DIR="${CRATE_ROOT}/scripts/ci/runs"
export CRATE_ROOT MONOREPO_ROOT RUNS_DIR

# Tunables (env override)
ACT_IMAGE="${ACT_IMAGE:-catthehacker/ubuntu:act-latest}"
# Default: do NOT pull every run (saves minutes). Set ACT_PULL=1 to refresh.
ACT_PULL="${ACT_PULL:-0}"
# Job wall-clock timeout (seconds) for act cargo builds that can hang Docker Desktop.
ACT_JOB_TIMEOUT="${ACT_JOB_TIMEOUT:-900}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
# Keep last N run dirs (tmp logs)
ACT_RUNS_KEEP="${ACT_RUNS_KEEP:-5}"

act_require() {
  command -v act >/dev/null 2>&1 || {
    echo "error: act not found (https://nektosact.com/ — brew install act)" >&2
    exit 127
  }
  command -v docker >/dev/null 2>&1 || {
    echo "error: docker not found; act needs a running Docker daemon" >&2
    exit 127
  }
  if ! docker info >/dev/null 2>&1; then
    echo "error: Docker daemon not responding — start Docker Desktop and retry" >&2
    exit 1
  fi
}

host_require() {
  command -v just >/dev/null 2>&1 || {
    echo "error: just not found (brew install just)" >&2
    exit 127
  }
  command -v cargo >/dev/null 2>&1 || {
    echo "error: cargo not found" >&2
    exit 127
  }
}

# Aggressive teardown: stop/remove act containers + named volumes (prevents Desktop lag).
act_teardown() {
  local quiet="${1:-}"
  log() { [[ -n "$quiet" ]] || echo "$*"; }

  log ">>> act teardown: containers / volumes"

  # act labels containers in various ways across versions
  local ids
  ids="$(
    {
      docker ps -aq --filter "label=com.github.nektos.act" 2>/dev/null || true
      docker ps -aq --filter "label=act" 2>/dev/null || true
      docker ps -aq --filter "name=act-" 2>/dev/null || true
      docker ps -aq --filter "name=^/act" 2>/dev/null || true
    } | sort -u | tr '\n' ' '
  )"

  if [[ -n "${ids// }" ]]; then
    log "    stopping: $ids"
    # shellcheck disable=SC2086
    docker stop -t 5 $ids >/dev/null 2>&1 || true
    # shellcheck disable=SC2086
    docker rm -f $ids >/dev/null 2>&1 || true
  else
    log "    no act containers"
  fi

  # Volumes created by act only (never match unrelated names like *contracts*)
  local vols
  vols="$(
    docker volume ls -q 2>/dev/null | grep -E '^(act-|act_|nektos)' || true
  )"
  if [[ -n "$vols" ]]; then
    log "    removing act volumes:"
    while IFS= read -r v; do
      [[ -z "$v" ]] && continue
      docker volume rm -f "$v" >/dev/null 2>&1 || true
      log "      - $v"
    done <<<"$vols"
  fi

  # Dangling anonymous leftovers from failed runs
  docker container prune -f >/dev/null 2>&1 || true
  docker network prune -f >/dev/null 2>&1 || true

  log ">>> act teardown done"
}

# Prune old run log directories; keep ACT_RUNS_KEEP newest + latest-status.md
act_prune_runs() {
  mkdir -p "${RUNS_DIR}"
  # shellcheck disable=SC2012
  local count
  count="$(ls -1d "${RUNS_DIR}"/*_tmp "${RUNS_DIR}"/*_ext_tmp 2>/dev/null | wc -l | tr -d ' ')"
  if [[ "${count:-0}" -le "${ACT_RUNS_KEEP}" ]]; then
    return 0
  fi
  ls -1dt "${RUNS_DIR}"/*_tmp "${RUNS_DIR}"/*_ext_tmp 2>/dev/null \
    | tail -n +"$((ACT_RUNS_KEEP + 1))" \
    | while IFS= read -r d; do
        rm -rf "$d"
      done
}

# Arch for Apple Silicon: native arm64 unless ACT_ARCH set
act_arch_args() {
  if [[ -n "${ACT_ARCH:-}" ]]; then
    echo "--container-architecture ${ACT_ARCH}"
    return
  fi
  local m
  m="$(uname -m 2>/dev/null || echo x86_64)"
  case "$m" in
    arm64|aarch64) echo "--container-architecture linux/arm64" ;;
    *) echo "" ;;
  esac
}

# Run one act job with wall-clock timeout + always teardown after.
# Usage: act_run_job <workflow_abs> <job> <log_file>
# Caller must `cd` to MONOREPO_ROOT first (sibling path deps).
act_run_job() {
  local workflow="$1"
  local job="$2"
  local log_file="$3"
  local rc=0
  local arch_flag
  arch_flag="$(act_arch_args)"

  act_require

  echo "    act -j ${job}  timeout=${ACT_JOB_TIMEOUT}s pull=${ACT_PULL} image=${ACT_IMAGE}"

  # Build argv as a string-safe list for bash 3.2
  local -a cmd=(
    act -W "${workflow}" -j "${job}"
    -P "ubuntu-latest=${ACT_IMAGE}"
    -b
    --rm
    --no-cache-server
    --env "CARGO_TERM_COLOR=always"
    --env "RUST_BACKTRACE=1"
    --env "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}"
    --env "GITHUB_WORKSPACE=${CRATE_ROOT}"
  )
  if [[ "${ACT_PULL}" == "1" ]]; then
    cmd+=(--pull)
  fi
  if [[ -n "${arch_flag}" ]]; then
    # shellcheck disable=SC2206
    cmd+=(${arch_flag})
  fi

  set +e
  if command -v gtimeout >/dev/null 2>&1; then
    gtimeout --signal=TERM --kill-after=30 "${ACT_JOB_TIMEOUT}" "${cmd[@]}" 2>&1 | tee "${log_file}"
    rc="${PIPESTATUS[0]}"
  elif command -v timeout >/dev/null 2>&1; then
    timeout --signal=TERM --kill-after=30 "${ACT_JOB_TIMEOUT}" "${cmd[@]}" 2>&1 | tee "${log_file}"
    rc="${PIPESTATUS[0]}"
  else
    "${cmd[@]}" 2>&1 | tee "${log_file}" &
    local pid=$!
    local elapsed=0
    while kill -0 "$pid" 2>/dev/null; do
      if [[ "$elapsed" -ge "${ACT_JOB_TIMEOUT}" ]]; then
        echo "error: act job ${job} exceeded ${ACT_JOB_TIMEOUT}s — killing" >&2
        kill -TERM "$pid" 2>/dev/null || true
        sleep 3
        kill -KILL "$pid" 2>/dev/null || true
        act_teardown quiet
        rc=124
        break
      fi
      sleep 2
      elapsed=$((elapsed + 2))
    done
    if [[ "$rc" -ne 124 ]]; then
      wait "$pid" 2>/dev/null
      rc=$?
    fi
  fi
  set -e

  act_teardown quiet
  return "$rc"
}

# Resolve workflow path for GHA files under crate .github/workflows
act_workflow_path() {
  local name="$1" # e.g. ci-core.yml
  echo "${CRATE_ROOT}/.github/workflows/${name}"
}

# Absolute workflow path for act -W when running from monorepo
act_workflow_for_act() {
  local name="$1"
  # Prefer monorepo-relative path if monorepo layout
  if [[ -f "${MONOREPO_ROOT}/crates/terp-rs/.github/workflows/${name}" ]]; then
    echo "${MONOREPO_ROOT}/crates/terp-rs/.github/workflows/${name}"
  else
    act_workflow_path "$name"
  fi
}

timed_step() {
  local label="$1"
  shift
  local start end
  start="$(date +%s)"
  echo ""
  echo "=== ${label} ==="
  set +e
  "$@"
  local rc=$?
  set -e
  end="$(date +%s)"
  echo "--- ${label}: exit=${rc} duration=$((end - start))s ---"
  return "$rc"
}
