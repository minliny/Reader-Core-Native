#!/usr/bin/env bash
# Repo-root wrapper for the corpus-batch-selector tool.
# Usage: scripts/corpus-batch-selector.sh [--manifest PATH | --root PATH] [--pretty] [--out PATH]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

exec python3 "${ROOT}/tools/corpus-batch-selector/corpus_batch_selector.py" "$@"
