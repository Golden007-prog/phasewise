#!/bin/sh
# Smoke test with a real agent CLI.
#
# Spends a little: one run of two trivial phases. Run it only with an agent CLI
# that is installed and logged in. It works in a new temporary repository and
# never touches this one.
#
# Usage: scripts/smoke-test.sh claude|codex [path/to/phasewise]
set -eu

agent=${1:-}
case "$agent" in
claude | codex) ;;
*)
    echo "usage: scripts/smoke-test.sh claude|codex [path/to/phasewise]" >&2
    exit 1
    ;;
esac
bin=${2:-"$(pwd)/target/debug/phasewise"}
if [ ! -x "$bin" ]; then
    echo "phasewise binary not found at $bin; run \`cargo build\` first or pass its path" >&2
    exit 1
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/phasewise-smoke.XXXXXX")
cd "$work"
git init -q -b main
echo "Smoke test repository." >README.md
cat >phasewise.md <<'PLAN'
# Smoke test

## Phase 1: Create a file
Create a file named hello.txt containing exactly the word: hello

```verify
test "$(cat hello.txt)" = "hello"
```

## Phase 2: Append a line
Append a second line to hello.txt containing exactly the word: world

```verify
test "$(sed -n 2p hello.txt)" = "world"
```
PLAN
git add -A
git commit -q -m "Smoke test setup"
base=$(git rev-parse main)

echo "== Repository: $work"
echo "== Phase 1"
"$bin" run --agent "$agent" || true
echo "== Phase 2"
"$bin" next || true
"$bin" status

fail=0
check() {
    if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fail=1; fi
}
run_id=$(cat .phasewise/current)
state=".phasewise/runs/$run_id/state.json"
check "phase 1 awaiting review or done" "grep -A3 '\"number\": 1' '$state' | grep -Eq '\"status\": \"(awaiting_review|done)\"'"
check "phase 2 awaiting review or done" "grep -A3 '\"number\": 2' '$state' | grep -Eq '\"status\": \"(awaiting_review|done)\"'"
check "two checkpoints" "[ \"\$(git log --format=%s | grep -c '^phasewise: phase')\" -eq 2 ]"
check "session id captured" "grep -Eq '\"session_id\": \"[^\"]+\"' '$state'"
check "phase 1 diff adds hello" "\"$bin\" diff 1 | grep -qx '+hello'"
check "phase 2 diff adds world" "\"$bin\" diff 2 | grep -qx '+world'"
check ".phasewise/ in no commit" "! git log --all --name-only --format= | grep -q '^\.phasewise'"
check "main unchanged" "[ \"\$(git rev-parse main)\" = \"$base\" ]"

echo "== Raw agent output (review and replace personal data before using as fixtures):"
ls "$work/.phasewise/runs/$run_id/"*.jsonl
exit "$fail"
