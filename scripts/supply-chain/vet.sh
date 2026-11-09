#!/usr/bin/env bash
# Mozilla cargo-vet for every root in roots.txt.
# --regenerate rewrites supply-chain/imports.lock and the exemptions table
# after Cargo.lock changes. The plain invocation is the gate.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
terp="$(cd "$here/../.." && pwd)"
regen=0
if [[ "${1:-}" == "--regenerate" ]]; then
  regen=1
elif [[ $# -gt 0 ]]; then
  echo "usage: $0 [--regenerate]" >&2
  exit 2
fi
if ! command -v cargo-vet >/dev/null 2>&1; then
  echo "cargo-vet is not installed. cargo install cargo-vet --locked" >&2
  exit 1
fi
# Keep metadata off the optimizer volume and the host target dir.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/terp-supply-chain-target}"
fail=0
while IFS= read -r rel || [[ -n "${rel:-}" ]]; do
  [[ -z "${rel}" || "${rel}" == \#* ]] && continue
  root="$terp/$rel"
  echo "== cargo vet  $root"
  if [[ ! -f "$root/supply-chain/config.toml" ]]; then
    echo "missing $root/supply-chain/config.toml" >&2
    echo "run: just supply vet-regenerate" >&2
    fail=1
    continue
  fi
  # `regenerate` does not accept --cargo-arg. The check does: keep it from
  # rewriting Cargo.lock while another cargo (the optimizer) is using the tree.
  if [[ "$regen" -eq 1 ]]; then
    if ! (cd "$root" && cargo vet regenerate audit-as-crates-io && cargo vet regenerate imports && cargo vet regenerate exemptions); then
      fail=1
      continue
    fi
  fi
  # cargo vet already passes --locked to cargo metadata, so the lock is not rewritten.
  if ! (cd "$root" && cargo vet); then
    fail=1
  fi
done <"$here/roots.txt"
exit "$fail"
