#!/usr/bin/env bash
# Safely prune merged branches and stale worktrees. Dry run unless --apply.
#   tools/scripts/prune-branches.sh [--apply] [--keep <branch>]...
# Deletes: remote branches whose tip is an ancestor of origin/main (and that
# have no open PR), local branches merged into origin/main with no worktree,
# and `git worktree prune`. Never touches main, branches of open PRs, --keep
# branches, or any branch checked out in a worktree. Worktrees themselves and
# Docker volumes are NOT removed here: check them by hand (clean? pushed? idle
# for 48 h?) and remove with `git worktree remove` (never --force a dirty one).
set -euo pipefail
apply=0; keep=" main HEAD "
while [ $# -gt 0 ]; do
  case "$1" in
    --apply) apply=1 ;;
    --keep) shift; keep+="$1 " ;;
    *) echo "usage: $0 [--apply] [--keep <branch>]..." >&2; exit 2 ;;
  esac
  shift
done
git fetch origin --prune --quiet
for b in $(gh pr list --state open --limit 200 --json headRefName --jq '.[].headRefName' | tr -d '\r'); do keep+="$b "; done
checked_out=$(git worktree list --porcelain | sed -n 's#^branch refs/heads/##p' | tr -d '\r')
act() { if [ "$apply" = 1 ]; then "$@"; else echo "[dry-run] $*"; fi; }

for r in $(git for-each-ref --merged origin/main --format='%(refname:short)' refs/remotes/origin | tr -d '\r'); do
  b=${r#origin/}
  case "$keep" in *" $b "*) continue ;; esac
  [ "$r" = origin ] && continue
  tip=$(git rev-parse "$r")
  act git push origin "--force-with-lease=refs/heads/$b:$tip" ":refs/heads/$b"
done
for b in $(git for-each-ref --merged origin/main --format='%(refname:short)' refs/heads | tr -d '\r'); do
  case "$keep" in *" $b "*) continue ;; esac
  echo "$checked_out" | grep -qx "$b" && continue
  act git branch -d "$b"
done
act git worktree prune -v
[ "$apply" = 1 ] || echo "(dry run; pass --apply to do it)"
