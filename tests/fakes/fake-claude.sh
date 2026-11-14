#!/bin/sh
# Fake Claude Code for tests. Reads the prompt from stdin and emits stream-json.
# FAKE_MODE (or FAKE_MODE_<N> per phase): success, fail, fix_on_resume,
# never_fixes, commit, slow, branch. FAKE_LOG_DIR: append calls to calls.log.

prompt=$(cat)

resume=""
prev=""
for arg in "$@"; do
    if [ "$prev" = "--resume" ]; then
        resume=$arg
    fi
    prev=$arg
done

if [ -n "$resume" ]; then
    n=${resume#fake-session-}
else
    n=$(printf '%s\n' "$prompt" | sed -n 's/^## Your task: phase \([0-9][0-9]*\) of .*/\1/p' | head -n 1)
fi
sid="fake-session-$n"

eval "mode=\${FAKE_MODE_$n:-}"
if [ -z "$mode" ]; then
    mode=${FAKE_MODE:-success}
fi

if [ -n "$FAKE_LOG_DIR" ]; then
    {
        echo "=== call ==="
        echo "args: $*"
        echo "prompt:"
        printf '%s\n' "$prompt"
    } >> "$FAKE_LOG_DIR/calls.log"
fi

printf '{"type":"system","subtype":"init","session_id":"%s"}\n' "$sid"

if [ "$mode" = "slow" ]; then
    exec sleep 600
fi

file="phase-$n.txt"
case "$mode" in
    fix_on_resume)
        if [ -n "$resume" ]; then
            printf 'phase %s\n' "$n" > "$file"
        else
            printf 'broken\n' > "$file"
        fi
        ;;
    never_fixes)
        printf 'broken\n' > "$file"
        ;;
    commit)
        printf 'phase %s\n' "$n" > "$file"
        git add "$file"
        git commit -q -m "agent commit for phase $n"
        ;;
    branch)
        git switch -q -c "fake-agent-branch-$n"
        printf 'phase %s\n' "$n" > "$file"
        ;;
    fail)
        ;;
    *)
        printf 'phase %s\n' "$n" > "$file"
        ;;
esac

printf '{"type":"assistant","session_id":"%s","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"Working on phase %s."},{"type":"tool_use","id":"toolu_%s","name":"Write","input":{"file_path":"%s"}}]}}\n' "$sid" "$n" "$n" "$file"

if [ "$mode" = "fail" ]; then
    printf '{"type":"result","subtype":"error_during_execution","is_error":true,"result":"fake failure","session_id":"%s","total_cost_usd":0.001}\n' "$sid"
    exit 1
fi

printf '{"type":"result","subtype":"success","is_error":false,"result":"Phase %s done.","session_id":"%s","total_cost_usd":0.01}\n' "$n" "$sid"
