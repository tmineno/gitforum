#!/usr/bin/env bash
# Acceptance checks for the PTY driver: AT-10, AT-11 and AT-12 of
# doc/spec/TUI-UX-TESTING.md. Runs the sample key file twice.
#
# Usage: scripts/tui-ux/selftest.sh [git-forum binary]  (default target/debug/git-forum)
# Needs: uv, cargo (the driver writes its fixture with a cargo test), a built binary.
# Output: one ok/FAIL line per check; exit 0 only if every check passes.
# Side effects: writes under ./tmp/ only.
set -u
cd "$(dirname "$0")/../.."
bin=${1:-target/debug/git-forum}
keys=scripts/tui-ux/samples/enter-esc-q.keys
mkdir -p tmp
out=$(mktemp -d tmp/tui-ux-selftest-XXXXXX)
fail=0

check() {
  if "${@:2}"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi
}

drive() {
  uv run --quiet scripts/tui-ux/drive.py --bin "$bin" --fixture full --keys "$keys" --out "$1"
}

# Screens equal after masking dates (YYYY-MM-DD and ISO timestamps).
same_screens() {
  python3 - "$1" "$2" <<'EOF'
import re, sys
from pathlib import Path
a, b = Path(sys.argv[1]), Path(sys.argv[2])
mask = lambda p: re.sub(r"\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}(:\d{2})?Z?)?", "<date>", p.read_text())
names = sorted(p.name for p in a.glob("step-*.txt"))
ok = names == sorted(p.name for p in b.glob("step-*.txt")) and len(names) > 0
for n in names:
    if ok and mask(a / n) != mask(b / n):
        print(f"  differs: {n}", file=sys.stderr)
        ok = False
sys.exit(0 if ok else 1)
EOF
}

run_json_has() {
  python3 - "$1" <<'EOF'
import json, sys
r = json.load(open(sys.argv[1]))
ok = (r["keys"] == ["Enter", "Esc", "q"] and r["size"] == "80x24"
      and r["version"].startswith("git-forum ") and r["fixture"] == "full"
      and r["exit_code"] == 0 and r["timeout"] is False)
sys.exit(0 if ok else 1)
EOF
}

under_tmp() {
  python3 -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1]))["run_dir"].startswith("tmp/") else 1)' "$1"
}

git for-each-ref refs/forum > "$out/refs-before.txt"
drive "$out/a"; rc_a=$?
drive "$out/b"; rc_b=$?
git for-each-ref refs/forum > "$out/refs-after.txt"

check "AT-10 driver exits 0" test "$rc_a" -eq 0
check "AT-10 step-001.txt shows [esc/q]back" grep -qF "[esc/q]back" "$out/a/step-001.txt"
check "AT-10 step-002.txt shows [q]quit" grep -qF "[q]quit" "$out/a/step-002.txt"
check "AT-10 run.json records keys, size, version, fixture, exit code" run_json_has "$out/a/run.json"
check "AT-11 refs/forum of this repository unchanged" cmp -s "$out/refs-before.txt" "$out/refs-after.txt"
check "AT-11 the driver's repository is under ./tmp/" under_tmp "$out/a/run.json"
check "AT-12 second run exits 0" test "$rc_b" -eq 0
check "AT-12 two runs give the same screens (dates masked)" same_screens "$out/a" "$out/b"
# Control: the comparison must notice a changed screen. Copy run a with its
# step-000.txt replaced by step-001.txt.
cp -r "$out/a" "$out/control"
cp "$out/a/step-001.txt" "$out/control/step-000.txt"
differ() { ! same_screens "$@" 2>/dev/null; }
check "AT-12 control: a changed screen is reported" differ "$out/a" "$out/control"
echo "output: $out"
exit "$fail"
