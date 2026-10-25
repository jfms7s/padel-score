#!/bin/sh
# Woodpecker check: every non-merge commit made since the rule was adopted has a Conventional
# Commits subject, `type[(scope)][!]: description` (see CLAUDE.md). Commits older than
# COMMIT_SUBJECTS_SINCE are history and never checked, so branches cut before the rule still
# pass. The repo is private and only the clone step gets credentials, so there is no main ref to
# diff against: the check walks HEAD's own history (the clone is full, `partial: false`). Commits
# that came from main were checked on their own branch already and pass again. Git's default
# `Revert "..."` subject is accepted. Tests pipe "<sha> <subject>" lines in with
# COMMIT_SUBJECTS_STDIN=1 instead of reading git.
set -eu
SINCE="${COMMIT_SUBJECTS_SINCE:-2026-10-17T00:00:00Z}"
types='feat|fix|docs|chore|refactor|build|ci|style|perf|test|revert'
pattern="^($types)(\([A-Za-z0-9._/-]+\))?!?: [^ ]"

if [ "${COMMIT_SUBJECTS_STDIN:-}" = 1 ]; then
  lines="$(cat)"
else
  if [ "$(git rev-parse --is-shallow-repository)" = true ]; then
    echo "commit-subjects: the clone is shallow; set partial: false on the clone step" >&2; exit 1
  fi
  lines="$(git log --no-merges --since="$SINCE" --format='%h %s' HEAD)"
fi

bad=0
checked=0
while IFS= read -r line; do
  [ -n "$line" ] || continue
  checked=$((checked + 1))
  subject="${line#* }"
  if ! printf '%s\n' "$subject" | grep -Eq "$pattern|^Revert \""; then
    [ "$bad" -gt 0 ] || echo "commit-subjects: these subjects aren't Conventional Commits (type[(scope)][!]: description):" >&2
    echo "  $line" >&2
    bad=$((bad + 1))
  fi
done <<EOF
$lines
EOF

if [ "$bad" -gt 0 ]; then
  echo "commit-subjects: $bad of $checked failed; types: ${types}. Fix with git rebase -i (reword) and force-push the branch." >&2
  exit 1
fi
echo "commit-subjects: $checked commit(s) since $SINCE, all conventional"
