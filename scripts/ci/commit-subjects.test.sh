#!/usr/bin/env bash
# Tests commit-subjects.sh on piped subjects: every Conventional Commits shape passes, a plain
# English subject or a malformed header fails, and nothing to check passes.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fail=0
check() { # check <pass|fail> <subject>
  local rc=0
  printf 'abc1234 %s\n' "$2" | COMMIT_SUBJECTS_STDIN=1 ${SH:-sh} "$here/commit-subjects.sh" >/dev/null 2>&1 || rc=$?
  if [[ "$1" == pass && $rc -ne 0 ]]; then echo "FAIL: should pass: $2"; fail=1; fi
  if [[ "$1" == fail && $rc -eq 0 ]]; then echo "FAIL: should fail: $2"; fail=1; fi
}
check pass "feat: add the menu planner"
check pass "fix(telemetry): bound span names to standard methods"
check pass "feat(api)!: drop the v0 recipe endpoint"
check pass "ci: cache release layers in a ghcr.io registry cache, not RustFS"
check pass "refactor(web/recipes): split the editor"
check pass 'Revert "feat: add the menu planner"'
check fail "A merge refuses a backup recipe whose body Tomperô can't read"
check fail "Game: brigade ranks and Michelin star levels"
check fail "feat:no space after the colon"
check fail "feat: "
check fail "Feat: capitalised type"
check fail "feature: unknown type"
check fail "fix (scope): space before the scope"
check fail "fixup! feat: add the menu planner"

# One bad subject among good ones fails the whole run; nothing to check passes.
rc=0
printf 'a1 feat: one\nb2 Plain English\nc3 fix: three\n' | COMMIT_SUBJECTS_STDIN=1 ${SH:-sh} "$here/commit-subjects.sh" >/dev/null 2>&1 || rc=$?
[[ $rc -ne 0 ]] || { echo "FAIL: one bad subject among good ones must fail"; fail=1; }
rc=0
printf '' | COMMIT_SUBJECTS_STDIN=1 ${SH:-sh} "$here/commit-subjects.sh" >/dev/null 2>&1 || rc=$?
[[ $rc -eq 0 ]] || { echo "FAIL: no commits to check must pass"; fail=1; }
exit $fail
