#!/bin/sh
# conclave lane commit guard (ADR 0008, Lane C) — installed by
# `conclave lane guard install` into the SHARED checkout's hooks.
#
# Purpose: reject commits whose staged paths fall outside
# $CONCLAVE_COMMIT_SCOPE, making the b9ab709 mixed-commit accident class
# impossible — a stray `git commit` in the shared tree can no longer sweep
# another lane's staged work.
#
# LOAD-BEARING self-skip: git shares one hooks directory across every linked
# worktree via the common git dir, so this same script also runs inside lane
# worktrees. Without the skip below it would block every lane commit. In a
# linked worktree `--git-dir` differs from `--git-common-dir`; in the shared
# checkout they resolve to the same path. Keep this POSIX sh.

# --- self-skip in lane worktrees ------------------------------------------
# git runs pre-commit from the worktree top with a consistent --git-dir form:
# in the shared checkout both --git-dir and --git-common-dir report ".git"
# (equal → guard runs); in a linked worktree --git-dir is an absolute path
# ".../.git/worktrees/<name>" while --git-common-dir is ".../.git" (differ →
# skip). Compare the RAW outputs — do NOT canonicalise; resolving symlinks
# (e.g. macOS /tmp → /private/tmp, /var → /private/var) would desync the two
# and make the shared checkout falsely skip.
gd=$(git rev-parse --git-dir 2>/dev/null) || exit 0
common=$(git rev-parse --git-common-dir 2>/dev/null) || exit 0
[ "$gd" != "$common" ] && exit 0

# --- shared checkout: a commit scope is mandatory -------------------------
if [ -z "$CONCLAVE_COMMIT_SCOPE" ]; then
  echo "conclave lane guard: CONCLAVE_COMMIT_SCOPE is unset." >&2
  echo "  The shared checkout requires an explicit commit scope so a stray" >&2
  echo "  'git commit' cannot sweep another lane's staged work (b9ab709)." >&2
  echo "  Set it to the comma-separated path prefixes you own, e.g.:" >&2
  echo "    CONCLAVE_COMMIT_SCOPE=docs/adr,src-tauri/src/engine git commit ..." >&2
  echo "  Lane worktrees under .claude/worktrees/ are exempt automatically." >&2
  exit 1
fi

# --- every staged path must fall under one scope prefix -------------------
staged=$(git diff --cached --name-only)
[ -z "$staged" ] && exit 0

# Return 0 if $1 (a staged path) is inside any comma-separated scope entry.
# An entry matches a path that equals it or lives under it ("$entry"/*), so a
# scope of "src" never matches "srcfoo". A trailing slash on an entry is
# stripped so "docs/" and "docs" behave identically.
in_scope() {
  _p=$1
  _oldifs=$IFS
  IFS=,
  for _e in $CONCLAVE_COMMIT_SCOPE; do
    IFS=$_oldifs
    case "$_e" in */) _e=${_e%/} ;; esac
    if [ -n "$_e" ]; then
      case "$_p" in
        "$_e"|"$_e"/*)
          IFS=$_oldifs
          return 0
          ;;
      esac
    fi
    IFS=,
  done
  IFS=$_oldifs
  return 1
}

# A here-doc redirect (not a pipe) keeps the loop in the current shell, so rc
# survives it — a `... | while` runs the loop in a subshell and loses rc.
rc=0
while IFS= read -r path; do
  [ -n "$path" ] || continue
  if ! in_scope "$path"; then
    echo "conclave lane guard: refusing commit — '$path' is outside CONCLAVE_COMMIT_SCOPE ($CONCLAVE_COMMIT_SCOPE)." >&2
    rc=1
  fi
done <<EOF
$staged
EOF

if [ "$rc" -ne 0 ]; then
  echo "  Stage only paths within your lane's scope, or adjust CONCLAVE_COMMIT_SCOPE." >&2
fi
exit $rc
