# /// script
# requires-python = ">=3.10"
# dependencies = ["pexpect==4.9.0", "pyte==0.8.2"]
# ///
"""Drive `git-forum tui` on a PTY from a key file and save every screen.

Spec: doc/spec/TUI-UX-TESTING.md, part B. Procedure: doc/tui-ux-review.md.

Preconditions: a built git-forum binary; cargo, to write the fixture
    repository (the same one the TUI test suites use); run from the repo.
Postconditions: OUT holds step-000.txt (the screen after start-up), one
    step-NNN.txt per operation, step-NNN.hl.txt beside each (the cells drawn
    with a background colour or reversed, which is how the TUI shows a
    selection), and run.json.
Failure modes: exit 2 for a bad argument or key file; exit 1 when a screen
    does not settle in time (run.json: timeout) or the TUI exits non-zero.
Side effects: creates a throwaway repository under ./tmp/ for every run.
    Never opens an existing repository, and gives the TUI a PATH without
    clipboard commands, a no-op editor, and HOME / TMPDIR inside the run.
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

import pexpect
import pyte

ROOT = Path(__file__).resolve().parents[2]

# A screen counts as settled once its text and highlights have not changed
# for SETTLE_MS (the TUI keeps writing cursor sequences even when nothing
# changes, so silence on the PTY never comes); one operation may take at
# most DEADLINE_S. See the spec for the measurement.
SETTLE_MS = 300
DEADLINE_S = 10.0

KEYS = {
    "Enter": "\r", "Esc": "\x1b", "Tab": "\t", "BackTab": "\x1b[Z",
    "Backspace": "\x7f", "Delete": "\x1b[3~", "Space": " ",
    "Up": "\x1b[A", "Down": "\x1b[B", "Right": "\x1b[C", "Left": "\x1b[D",
    "Home": "\x1b[H", "End": "\x1b[F", "PageUp": "\x1b[5~", "PageDown": "\x1b[6~",
}
# The TUI's clipboard helpers; none of them may be on the driver's PATH.
CLIPBOARD = ("pbcopy", "wl-copy", "xclip", "xsel")


class KeyFileError(ValueError):
    pass


def point(text, line_no):
    try:
        x, y = (int(v) for v in text.split(","))
    except ValueError:
        raise KeyFileError(f"line {line_no}: expected <x>,<y>, got {text!r}") from None
    return x, y


def sgr(button, x, y, press):
    # SGR mouse report; the screen files count from 0, the terminal from 1.
    return f"\x1b[<{button};{x + 1};{y + 1}{'M' if press else 'm'}"


def parse(line, line_no):
    """One key-file line -> (label, bytes to send or None, resize or None)."""
    if line.startswith("text:"):
        return line, line[len("text:"):], None
    if line.startswith("resize:"):
        m = re.fullmatch(r"resize:(\d+)x(\d+)", line)
        if not m:
            raise KeyFileError(f"line {line_no}: expected resize:<W>x<H>, got {line!r}")
        return line, None, (int(m.group(1)), int(m.group(2)))
    for kind, clicks in (("click:", 1), ("dblclick:", 2)):
        if line.startswith(kind):
            x, y = point(line[len(kind):], line_no)
            return line, (sgr(0, x, y, True) + sgr(0, x, y, False)) * clicks, None
    m = re.fullmatch(r"scroll:(up|down)@(.*)", line)
    if m:
        x, y = point(m.group(2), line_no)
        return line, sgr(64 if m.group(1) == "up" else 65, x, y, True), None
    m = re.fullmatch(r"Ctrl-([a-z])", line)
    if m:
        return line, chr(ord(m.group(1)) - ord("a") + 1), None
    if line in KEYS:
        return line, KEYS[line], None
    if len(line) == 1:
        return line, line, None
    raise KeyFileError(f"line {line_no}: unknown operation {line!r}")


def read_keys(path):
    ops = []
    for n, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.rstrip("\n")
        if not line.strip() or line.startswith("#"):
            continue
        ops.append(parse(line, n))
    return ops


def export_fixture(fixture, repo):
    env = {k: v for k, v in os.environ.items()
           if k not in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE")}
    env.update(TUI_UX_EXPORT_FIXTURE=fixture, TUI_UX_EXPORT_DIR=str(repo))
    test = "internal::tui::ux_fixture::export_fixture"
    p = subprocess.run(
        ["cargo", "test", "--quiet", "--lib", test, "--", "--ignored", "--exact", test],
        cwd=ROOT, env=env, capture_output=True, text=True)
    if p.returncode != 0 or not (repo / ".git").is_dir():
        sys.exit(f"drive: writing the {fixture} fixture failed:\n{p.stdout}{p.stderr}")


def private_bin(run, forum_bin):
    """A PATH directory with git, git-forum and a no-op editor only."""
    bindir = run / "bin"
    bindir.mkdir()
    for name, target in (("git", shutil.which("git")), ("git-forum", forum_bin),
                         ("true", shutil.which("true"))):
        if not target:
            sys.exit(f"drive: {name} not found")
        (bindir / name).symlink_to(Path(target).resolve())
    assert not any((bindir / c).exists() for c in CLIPBOARD)
    return bindir


class Term:
    """The TUI on a PTY, mirrored into a pyte screen."""

    def __init__(self, forum_bin, repo, env, cols, rows, stderr_path, settle_ms):
        self.settle_ms = settle_ms
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.Stream(self.screen)

        def stderr_to_file():
            # Runs in the child, after ptyprocess has closed inherited fds.
            os.dup2(os.open(stderr_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644), 2)

        self.child = pexpect.spawn(
            str(forum_bin), ["tui"], cwd=str(repo), env=env, encoding="utf-8",
            codec_errors="replace", dimensions=(rows, cols), preexec_fn=stderr_to_file)

    def settle(self):
        """Read until the screen text and highlights have not changed for
        settle_ms. Returns (ms from the start of the wait to the last change,
        timed_out)."""
        start = last = time.monotonic()
        shown = self.snapshot()
        while True:
            now = time.monotonic()
            if now - start > DEADLINE_S:
                return round((last - start) * 1000), True
            if (now - last) * 1000 >= self.settle_ms:
                return round((last - start) * 1000), False
            try:
                data = self.child.read_nonblocking(65536, timeout=0.02)
            except pexpect.TIMEOUT:
                continue
            except pexpect.EOF:
                return round((last - start) * 1000), False
            self.answer_queries(data)
            self.stream.feed(data)
            if self.snapshot() != shown:
                shown = self.snapshot()
                last = time.monotonic()

    def answer_queries(self, data):
        # crossterm may ask for the cursor position or the device attributes.
        if "\x1b[6n" in data:
            y, x = self.screen.cursor.y, self.screen.cursor.x
            self.child.send(f"\x1b[{y + 1};{x + 1}R")
        if "\x1b[c" in data or "\x1b[0c" in data:
            self.child.send("\x1b[?1;2c")

    def resize(self, cols, rows):
        self.screen.resize(rows, cols)
        self.child.setwinsize(rows, cols)

    def text(self):
        return "\n".join(line.rstrip() for line in self.screen.display) + "\n"

    def highlights(self):
        """One line per run of cells drawn with a background colour or
        reversed: `<row>,<first col>-<last col>: <text>`."""
        def lit(cell):
            return cell.bg != "default" or cell.reverse

        spans = []
        for y in range(self.screen.lines):
            row = self.screen.buffer[y]
            x = 0
            while x < self.screen.columns:
                if not lit(row[x]):
                    x += 1
                    continue
                x0 = x
                while x < self.screen.columns and lit(row[x]):
                    x += 1
                text = "".join(row[i].data for i in range(x0, x)).rstrip()
                spans.append(f"{y},{x0}-{x - 1}: {text}\n")
        return "".join(spans)

    def snapshot(self):
        return self.text(), self.highlights()

    def save(self, out, step):
        (out / f"step-{step:03d}.txt").write_text(self.text())
        (out / f"step-{step:03d}.hl.txt").write_text(self.highlights())


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bin", required=True, type=Path, help="the git-forum binary")
    ap.add_argument("--fixture", required=True, choices=["full", "empty"])
    ap.add_argument("--size", default="80x24", help="terminal size WxH (default 80x24)")
    ap.add_argument("--keys", required=True, type=Path, help="key file, one operation a line")
    ap.add_argument("--out", required=True, type=Path, help="output directory (must not exist)")
    ap.add_argument("--settle-ms", type=int, default=SETTLE_MS,
                    help=f"quiet time that ends a step (default {SETTLE_MS}; for measuring)")
    args = ap.parse_args()

    size = re.fullmatch(r"(\d+)x(\d+)", args.size)
    if not size:
        ap.error("--size must be WxH")
    cols, rows = int(size.group(1)), int(size.group(2))
    if not args.bin.is_file():
        ap.error(f"--bin {args.bin} is not a file")
    if args.out.exists():
        ap.error(f"--out {args.out} already exists")
    try:
        ops = read_keys(args.keys)
    except (OSError, KeyFileError) as e:
        print(f"drive: {e}", file=sys.stderr)
        sys.exit(2)

    (ROOT / "tmp").mkdir(exist_ok=True)
    run = Path(tempfile.mkdtemp(prefix="tui-ux-", dir=ROOT / "tmp"))
    repo = run / "repo"
    export_fixture(args.fixture, repo)
    for d in ("home", "tmp"):
        (run / d).mkdir()
    env = {
        "PATH": str(private_bin(run, args.bin)), "HOME": str(run / "home"),
        "TMPDIR": str(run / "tmp"), "TERM": "xterm-256color",
        "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
        "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_FORUM_ACTOR": "ai/tui-ux-review", "EDITOR": "true", "VISUAL": "true",
    }
    version = subprocess.run([str(args.bin), "--version"], env=env, capture_output=True,
                             text=True).stdout.strip()

    args.out.mkdir(parents=True)
    stderr_path = run / "stderr.txt"
    term = Term(args.bin.resolve(), repo, env, cols, rows, stderr_path, args.settle_ms)
    steps, timed_out = [], False
    last_ms, timed_out = term.settle()
    term.save(args.out, 0)
    steps.append({"step": 0, "op": None, "last_change_ms": last_ms})
    for i, (label, data, new_size) in enumerate(ops, 1):
        if timed_out or not term.child.isalive():
            break
        if new_size:
            term.resize(*new_size)
        else:
            term.child.send(data)
        last_ms, timed_out = term.settle()
        term.save(args.out, i)
        steps.append({"step": i, "op": label, "last_change_ms": last_ms})

    quit_by_driver = term.child.isalive() and not timed_out
    if quit_by_driver:
        # INV-4: at most two Ctrl-C quit from any state.
        for _ in range(2):
            if term.child.isalive():
                term.child.send("\x03")
                term.settle()
    term.child.close(force=True)
    exit_code = term.child.exitstatus
    if exit_code is None and term.child.signalstatus is not None:
        exit_code = -term.child.signalstatus

    def rel(p):
        p = Path(p).resolve()
        return str(p.relative_to(ROOT)) if p.is_relative_to(ROOT) else p.name

    record = {
        "keys": [label for label, _, _ in ops],
        "keys_file": rel(args.keys),
        "size": f"{cols}x{rows}",
        "fixture": args.fixture,
        "binary": rel(args.bin),
        "version": version,
        "run_dir": rel(run),
        "steps_run": len(steps) - 1,
        "exit_code": exit_code,
        "quit_by_driver": quit_by_driver,
        "stderr": stderr_path.read_text(errors="replace"),
        "timeout": timed_out,
        "settle_ms": args.settle_ms,
        "deadline_s": DEADLINE_S,
        "steps": steps,
    }
    (args.out / "run.json").write_text(json.dumps(record, ensure_ascii=False, indent=1) + "\n")
    if timed_out:
        print(f"drive: screen did not settle within {DEADLINE_S}s at step {len(steps) - 1}",
              file=sys.stderr)
        sys.exit(1)
    if exit_code not in (0, None):
        print(f"drive: the TUI exited with {exit_code}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
