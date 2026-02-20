#!/usr/bin/env bash
set -euo pipefail

crates=(proto gateway agent tools channels skills cli)
threshold=85
failed=0

for crate in "${crates[@]}"; do
  echo "==> checking rustdoc coverage for ${crate}"
  output=$(RUSTDOCFLAGS='-Z unstable-options --show-coverage' cargo +nightly doc -p "${crate}" --no-deps 2>&1)
  echo "${output}"

  total_line=$(printf '%s\n' "${output}" | rg '^\| Total\s+\|' | tail -n 1 || true)
  if [[ -z "${total_line}" ]]; then
    echo "ERROR: failed to parse rustdoc coverage output for ${crate}" >&2
    failed=1
    continue
  fi

  percent=$(printf '%s\n' "${total_line}" | awk -F'|' '{gsub(/%/, "", $4); gsub(/ /, "", $4); print $4}')
  if [[ -z "${percent}" ]]; then
    echo "ERROR: failed to parse coverage percentage for ${crate}" >&2
    failed=1
    continue
  fi

  # Compare as decimal using awk for portability.
  if ! awk -v p="${percent}" -v t="${threshold}" 'BEGIN { exit !(p + 0 >= t + 0) }'; then
    echo "FAIL: ${crate} rustdoc coverage ${percent}% < ${threshold}%" >&2
    failed=1
  else
    echo "PASS: ${crate} rustdoc coverage ${percent}% >= ${threshold}%"
  fi

done

if [[ "${failed}" -ne 0 ]]; then
  exit 1
fi
