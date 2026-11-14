#!/bin/sh
# Fake Codex for tests. Reads the prompt from stdin and emits `--json` events.
# FAKE_MODE (or FAKE_MODE_<N> per phase): success, fail, fix_on_resume,
# never_fixes, commit, slow, branch. FAKE_LOG_DIR: append calls to calls.log.

prompt=$(cat)

resume=""
prev=""
for arg in "$@"; do
    if [ "$prev" = "resume" ]; then
        resume=$arg
    fi
    prev=$arg
done

if [ -n "$resume" ]; then
    n=${resume#fake-thread-}
else
    n=$(printf '%s\n' "$prompt" | sed -n 's/^## Your task: phase \([0-9][0-9]*\) of .*/\1/p' | head -n 1)
fi
tid="fake-thread-$n"

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

printf '{"type":"thread.started","thread_id":"%s"}\n' "$tid"
echo '{"type":"turn.started"}'

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

printf '{"type":"item.completed","item":{"id":"item_1","type":"file_change","changes":[{"path":"%s","kind":"add"}],"status":"completed"}}\n' "$file"

if [ "$mode" = "fail" ]; then
    echo '{"type":"turn.failed","error":{"message":"fake failure"}}'
    exit 1
fi

printf '{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"Phase %s done."}}\n' "$n"
echo '{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}'
