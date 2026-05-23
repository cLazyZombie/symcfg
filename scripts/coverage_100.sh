#!/usr/bin/env bash
set -euo pipefail

cargo llvm-cov --version >/dev/null
command -v lcov_filter >/dev/null

cargo llvm-cov --workspace --all-targets --all-features --lcov --quiet \
  | lcov_filter --text
