#!/usr/bin/env bash
#
# Mirror the published speq-contracts schemas into tests/fixtures/contracts/.
#
# speq-cli must not own a private opinion of the contract. It vendors a copy so
# that `cargo test` works offline, and this script is what keeps that copy
# honest: it re-fetches the pinned revision from speq-contracts and overwrites
# the local files.
#
#   scripts/sync-contracts.sh           adopt the pinned revision (writes files)
#   scripts/sync-contracts.sh --check   fail if the local copy has drifted (CI)
#
# The pinned revision lives in tests/fixtures/contracts/CONTRACTS_PIN.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pin_file="$repo_root/tests/fixtures/contracts/CONTRACTS_PIN"
dest_root="$repo_root/tests/fixtures/contracts"

check_only=0
if [[ "${1:-}" == "--check" ]]; then
  check_only=1
elif [[ $# -gt 0 ]]; then
  echo "usage: $(basename "$0") [--check]" >&2
  exit 2
fi

[[ -f "$pin_file" ]] || { echo "missing $pin_file" >&2; exit 2; }

read_pin() {
  # Strip comments and blank lines, then take the value of key "$1".
  sed -e 's/#.*//' -e '/^[[:space:]]*$/d' "$pin_file" \
    | awk -F= -v key="$1" '$1 == key { sub(/^[^=]*=/, ""); gsub(/[[:space:]]/, ""); print; exit }'
}

repo="$(read_pin repo)"
ref="$(read_pin ref)"
schemas="$(read_pin schemas)"

for key in repo ref schemas; do
  [[ -n "$(read_pin "$key")" ]] || { echo "$pin_file: missing '$key='" >&2; exit 2; }
done

status=0
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

IFS=',' read -r -a schema_list <<< "$schemas"
for schema in "${schema_list[@]}"; do
  url="https://raw.githubusercontent.com/$repo/$ref/schemas/$schema"
  tmp_file="$tmp_dir/$(echo "$schema" | tr '/' '_')"

  if ! curl -fsSL "$url" -o "$tmp_file"; then
    echo "failed to fetch $url" >&2
    exit 3
  fi

  dest="$dest_root/$schema"
  mkdir -p "$(dirname "$dest")"

  if [[ $check_only -eq 1 ]]; then
    if [[ ! -f "$dest" ]]; then
      echo "DRIFT: $schema is missing from tests/fixtures/contracts/" >&2
      status=1
    elif ! diff -u "$dest" "$tmp_file" >/dev/null; then
      echo "DRIFT: tests/fixtures/contracts/$schema differs from $repo@$ref" >&2
      diff -u "$dest" "$tmp_file" >&2 || true
      status=1
    else
      echo "ok: $schema matches $repo@${ref:0:12}"
    fi
  else
    cp "$tmp_file" "$dest"
    echo "synced: $schema from $repo@${ref:0:12}"
  fi
done

if [[ $status -ne 0 ]]; then
  cat >&2 <<'MSG'

The vendored contract copy no longer matches the published one.
Do not hand-edit the files under tests/fixtures/contracts/. Either run
`scripts/sync-contracts.sh` to adopt the pinned revision, or, if the contract
itself should change, change it in speq-contracts first and move the pin.
MSG
fi

exit $status
