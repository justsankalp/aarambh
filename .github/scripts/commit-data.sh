#!/usr/bin/env bash
# Usage: commit-data.sh "<message>" <path>...
# Commits changes under the given paths and pushes to main. The app writes
# notes to main at any time, so a rejected push is rebased and retried.
set -euo pipefail
message="$1"
shift
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add -- "$@"
if git diff --cached --quiet; then
  echo "Nothing to commit."
  exit 0
fi
git commit -q -m "$message"
for attempt in 1 2 3 4 5; do
  if git push -q origin HEAD:main; then
    echo "Pushed."
    exit 0
  fi
  sleep $((attempt * 3))
  git pull -q --rebase origin main
done
echo "Could not push after 5 attempts." >&2
exit 1
