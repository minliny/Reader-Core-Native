#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: scripts/integration-queue.sh <integration-branch> <base-ref> <source-ref>...

Creates or reuses a sibling worktree, merges source refs in order, and runs the
local integration gates. It intentionally stops on dirty worktrees or merge
conflicts so agent branches can be integrated as soon as they produce a commit.

Environment:
  WORKTREE_DIR  Override the sibling worktree path.
  RUN_OHOS=1    Also run scripts/build-ohos.sh.
  RUN_NAPI=1    Also run scripts/build-harmony-napi.sh. Requires OHOS_SDK_HOME.
  PUSH=1        Push the integration branch after all gates pass.

Example:
  scripts/integration-queue.sh \
    codex/core-product-integration \
    origin/codex/core-foundation-integration \
    origin/codex/remote-reading-vertical
EOF
}

if [[ $# -lt 3 ]]; then
  usage
  exit 64
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
branch="$1"
base_ref="$2"
shift 2
sources=("$@")

safe_branch="${branch//\//-}"
worktree_dir="${WORKTREE_DIR:-"$repo_root/../Reader-Core-Native-$safe_branch"}"

cd "$repo_root"
git fetch origin

if [[ -d "$worktree_dir/.git" || -f "$worktree_dir/.git" ]]; then
  cd "$worktree_dir"
else
  if git show-ref --verify --quiet "refs/heads/$branch"; then
    git worktree add "$worktree_dir" "$branch"
  else
    git worktree add "$worktree_dir" -b "$branch" "$base_ref"
  fi
  cd "$worktree_dir"
fi

current_branch="$(git branch --show-current)"
if [[ "$current_branch" != "$branch" ]]; then
  echo "worktree is on $current_branch, expected $branch: $worktree_dir" >&2
  exit 65
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "worktree is dirty; refusing to integrate into $branch" >&2
  git status --short
  exit 66
fi

git fetch origin

for source in "${sources[@]}"; do
  echo "==> merging $source into $branch"
  git merge --no-ff --no-edit "$source"
done

echo "==> running local gates"
./scripts/check-local.sh
./scripts/build-local.sh

if [[ "${RUN_OHOS:-0}" == "1" ]]; then
  ./scripts/build-ohos.sh
fi

if [[ "${RUN_NAPI:-0}" == "1" ]]; then
  ./scripts/build-harmony-napi.sh
fi

if [[ "${PUSH:-0}" == "1" ]]; then
  git push -u origin "$branch"
fi

echo "integration branch ready: $branch"
git log --oneline --decorate -5
