#!/usr/bin/env bash
#
# shots-set.sh — one docs/screenshots/real set per release, on main only once it is everyone's.
#
# Every release gets its own `pnpm shots:real` capture. The set is held on a local branch,
# shots/vX.Y.Z, and reaches main only when vX.Y.Z goes to everyone: release.sh merges it in
# a normal release and after `./release.sh promote X.Y.Z`. An early build's screenshots
# therefore never show up in the README or on nixonapp.com while everyone is on an older
# version. release.sh uses this script; it is also safe to run by hand.
#
# Usage:
#   scripts/shots-set.sh park X.Y.Z [--replace]
#       Commit the working tree's docs/screenshots/real onto shots/vX.Y.Z (based on HEAD),
#       then reset that folder to HEAD. Never switches branches. --replace overwrites an
#       existing set (a recapture).
#   scripts/shots-set.sh has X.Y.Z            exit 0 if shots/vX.Y.Z exists
#   scripts/shots-set.sh export X.Y.Z DIR     write the set to DIR/docs/screenshots/real
#   scripts/shots-set.sh merge X.Y.Z [--push] [--dry-run]
#       Merge shots/vX.Y.Z into main. On main it is a normal merge; on any other branch
#       main is updated without a checkout. --push pushes main only when main was level with
#       origin/main beforehand, so nothing unreleased goes out with it.
#   scripts/shots-set.sh --self-test
set -euo pipefail

SHOTS_PATH="docs/screenshots/real"
die() { printf '\033[0;31m❌ %s\033[0m\n' "$*" >&2; exit 1; }
say() { printf '\033[0;34m%s\033[0m\n' "$*"; }
ok() { printf '\033[0;32m%s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m%s\033[0m\n' "$*"; }

ver_ok() { [[ "${1:-}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "expected a version X.Y.Z, got '${1:-}'"; }
ref_of() { printf 'refs/heads/shots/v%s' "$1"; }
has_set() { git rev-parse -q --verify "$(ref_of "$1")" >/dev/null; }

cmd_park() {
  local v="$1" replace="${2:-}" idx tree commit
  ver_ok "$v"
  if has_set "$v" && [ "$replace" != "--replace" ]; then
    die "shots/v${v} already exists. Recaptured? Re-run with --replace."
  fi
  [ -n "$(git status --porcelain -- "$SHOTS_PATH")" ] \
    || die "${SHOTS_PATH} has no changes against HEAD. Run pnpm shots:real first."
  idx="$(mktemp)"
  GIT_INDEX_FILE="$idx" git read-tree HEAD
  GIT_INDEX_FILE="$idx" git add -A -- "$SHOTS_PATH"
  tree="$(GIT_INDEX_FILE="$idx" git write-tree)"
  rm -f "$idx"
  commit="$(git commit-tree "$tree" -p HEAD -m "docs(screenshots): real set for v${v}")"
  git update-ref "$(ref_of "$v")" "$commit"
  # The set is safe on its branch; take it out of this working tree so it can't be committed
  # here by accident.
  git checkout -q HEAD -- "$SHOTS_PATH"
  git clean -fdq -- "$SHOTS_PATH"
  ok "✅ Parked $(git diff --name-only HEAD "$commit" | wc -l | tr -d ' ') screenshot files on shots/v${v} ($(git rev-parse --short "$commit")). It goes to main when v${v} reaches everyone."
}

cmd_export() {
  local v="$1" dir="$2"
  ver_ok "$v"
  [ -n "$dir" ] || die "export needs a target directory"
  has_set "$v" || die "no screenshot set shots/v${v}"
  mkdir -p "$dir"
  git archive "$(ref_of "$v")" "$SHOTS_PATH" | tar -x -C "$dir"
}

cmd_merge() {
  local v="$1" push=0 dry=0 arg old cur tree commit level=0
  shift
  for arg in "$@"; do
    case "$arg" in --push) push=1 ;; --dry-run) dry=1 ;; *) die "unexpected argument: $arg" ;; esac
  done
  ver_ok "$v"
  has_set "$v" || die "no screenshot set shots/v${v}"
  local set_ref msg
  set_ref="$(ref_of "$v")"
  msg="docs(screenshots): v${v} set"
  if git merge-base --is-ancestor "$set_ref" main; then
    ok "   screenshots: shots/v${v} is already on main."
    return 0
  fi
  old="$(git rev-parse main)"
  if git rev-parse -q --verify refs/remotes/origin/main >/dev/null \
     && [ "$old" = "$(git rev-parse refs/remotes/origin/main)" ]; then
    level=1
  fi
  if [ "$dry" -eq 1 ]; then
    echo "  [dry-run] merge shots/v${v} into main ('${msg}')"
    [ "$push" -eq 1 ] && echo "  [dry-run] push main (only if main is level with origin/main: $([ "$level" -eq 1 ] && echo yes || echo no))"
    return 0
  fi
  cur="$(git rev-parse --abbrev-ref HEAD)"
  if [ "$cur" = "main" ]; then
    [ -z "$(git status --porcelain -- "$SHOTS_PATH")" ] \
      || die "${SHOTS_PATH} has uncommitted changes on main; park or discard them, then re-run: scripts/shots-set.sh merge ${v}"
    git merge -q --no-ff -m "$msg" "$set_ref" \
      || die "merging shots/v${v} into main failed; resolve it, then re-run: scripts/shots-set.sh merge ${v}"
  else
    # main is not checked out here, so build the merge commit without touching any worktree.
    tree="$(git merge-tree --write-tree main "$set_ref")" \
      || die "shots/v${v} conflicts with main. Merge it by hand on main."
    commit="$(git commit-tree "$tree" -p main -p "$set_ref" -m "$msg")"
    git update-ref refs/heads/main "$commit" "$old"
  fi
  ok "   screenshots: merged shots/v${v} into main ($(git rev-parse --short main))."
  if [ "$push" -eq 1 ]; then
    if [ "$level" -eq 1 ]; then
      git push -q origin main || die "push of main failed; the merge is local. Re-run: git push origin main"
      ok "   screenshots: pushed main."
    else
      warn "   screenshots: main was not level with origin/main before the merge, so it was not pushed (that would publish other commits). Push it when you mean to: git push origin main"
    fi
  fi
}

self_test() {
  local t self
  self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  t="$(mktemp -d)"
  trap "rm -rf '$t'" EXIT
  (
    set -e
    fail() { echo "SELF-TEST FAIL: $*" >&2; exit 1; }
    cd "$t"
    git init -q -b main origin.git --bare
    git init -q -b main repo && cd repo
    git config user.email t@t; git config user.name t
    git remote add origin ../origin.git
    mkdir -p "$SHOTS_PATH"; echo old > "$SHOTS_PATH/a.png"; echo keep > "$SHOTS_PATH/gone.png"; echo r > README.md
    git add -A; git commit -qm base; git push -q origin main; git fetch -q origin

    # park: needs changes; captures edits, additions and deletions; resets the folder
    if bash "$self" park 1.0.0 2>/dev/null; then fail "park with no changes should refuse"; fi
    echo new > "$SHOTS_PATH/a.png"; echo add > "$SHOTS_PATH/b.png"; rm "$SHOTS_PATH/gone.png"
    bash "$self" park 1.0.0 >/dev/null
    bash "$self" has 1.0.0 || fail "has after park"
    [ -z "$(git status --porcelain)" ] || fail "park left changes in the tree"
    [ "$(git show shots/v1.0.0:$SHOTS_PATH/a.png)" = new ] || fail "set lacks the edit"
    git cat-file -e "shots/v1.0.0:$SHOTS_PATH/gone.png" 2>/dev/null && fail "set kept a deleted file"
    [ "$(git rev-parse --abbrev-ref HEAD)" = main ] || fail "park switched branches"
    echo again > "$SHOTS_PATH/a.png"
    if bash "$self" park 1.0.0 2>/dev/null; then fail "park over an existing set should need --replace"; fi
    bash "$self" park 1.0.0 --replace >/dev/null
    [ "$(git show shots/v1.0.0:$SHOTS_PATH/a.png)" = again ] || fail "--replace did not replace"

    # export
    bash "$self" export 1.0.0 "$t/out"
    [ "$(cat "$t/out/$SHOTS_PATH/a.png")" = again ] || fail "export"
    if bash "$self" export 9.9.9 "$t/out2" 2>/dev/null; then fail "export of a missing set should fail"; fi

    # merge from another branch: main moves, worktree untouched, pushed (main was level)
    git switch -qc feature; echo f > f.txt; git add f.txt; git commit -qm feature
    bash "$self" merge 1.0.0 --push >/dev/null
    [ "$(git show main:$SHOTS_PATH/a.png)" = again ] || fail "merge did not reach main"
    [ "$(git rev-parse --abbrev-ref HEAD)" = feature ] || fail "merge switched branches"
    [ "$(cat "$SHOTS_PATH/a.png")" = old ] || fail "merge touched the feature worktree"
    git fetch -q origin
    [ "$(git rev-parse origin/main)" = "$(git rev-parse main)" ] || fail "merge --push did not push"
    bash "$self" merge 1.0.0 | grep -q "already on main" || fail "re-merge should be a no-op"

    # merge on main; --push refuses when main is ahead of origin with other work
    git switch -q main
    echo n2 > "$SHOTS_PATH/a.png"; bash "$self" park 1.1.0 >/dev/null
    echo unreleased > u.txt; git add u.txt; git commit -qm unreleased
    bash "$self" merge 1.1.0 --push >/dev/null 2>&1
    [ "$(cat "$SHOTS_PATH/a.png")" = n2 ] || fail "merge on main did not update the worktree"
    git fetch -q origin
    [ "$(git rev-parse origin/main)" != "$(git rev-parse main)" ] || fail "--push pushed unreleased work"
  ) || exit 1
  ok "shots-set.sh self-test passed"
}

case "${1:-}" in
  park) shift; [ $# -ge 1 ] || die "usage: scripts/shots-set.sh park X.Y.Z [--replace]"; cmd_park "$@" ;;
  has) shift; ver_ok "${1:-}"; has_set "$1" ;;
  export) shift; cmd_export "${1:-}" "${2:-}" ;;
  merge) shift; [ $# -ge 1 ] || die "usage: scripts/shots-set.sh merge X.Y.Z [--push] [--dry-run]"; cmd_merge "$@" ;;
  --self-test) self_test ;;
  -h|--help|"") sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//' ;;
  *) die "unknown command: $1 (park | has | export | merge | --self-test)" ;;
esac
