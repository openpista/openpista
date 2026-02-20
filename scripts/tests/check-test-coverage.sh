#!/usr/bin/env bash
set -euo pipefail

cargo llvm-cov \
  --workspace \
  --all-targets \
  --summary-only \
  --fail-under-lines 85 \
  --fail-under-regions 85
