# VHS script that records demo/demo.gif.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), phasewise on the
# PATH, and a Claude Code CLI that is logged in. The demo runs a real agent
# on two tiny phases, so it spends a little.
#
# Run it from the repository root:  vhs demo/demo.tape

Output demo/demo.gif

Require phasewise
Require git
Require claude

Set Shell "bash"
Set FontSize 16
Set Width 1200
Set Height 700
Set TypingSpeed 60ms

Hide
Type `export DEMO=$(mktemp -d) && cd $DEMO && git init -q -b main && git config user.name demo && git config user.email demo@example.com` Enter
Type `printf '+++\nverify = ["test -f hello.txt"]\n+++\n\n# Hello demo\n\n## Phase 1: Create the file\n\nCreate hello.txt containing the word hello.\n\n## Phase 2: Extend the file\n\nAppend a second line with the word world to hello.txt.\n' > phasewise.md` Enter
Type `git add phasewise.md && git commit -q -m 'Add plan' && clear` Enter
Sleep 1s
Show

Type "phasewise check" Enter
Sleep 3s

Type "phasewise run" Enter
Sleep 90s

Type "phasewise status" Enter
Sleep 3s

Type "phasewise diff" Enter
Sleep 4s

Type "phasewise next" Enter
Sleep 90s

Type "phasewise review" Enter
Sleep 2s
Tab
Sleep 1s
Tab
Sleep 1s
Tab
Sleep 1s
Down
Sleep 1s
Up
Sleep 1s
Type "q"
Sleep 2s
