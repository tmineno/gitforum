"""Check an AI UX review's findings before they are reported.

Spec: doc/spec/TUI-UX-TESTING.md (part B, failure mode 9). Procedure:
doc/tui-ux-review.md.

Preconditions: FINDINGS is the review's findings.json; each finding names
    its key file, terminal size, fixture, step number and quoted text.
Postconditions: every finding's key file has been replayed with drive.py
    into OUT/<id>/, and OUT/verify.json holds one verdict a finding:
    "引用行あり" (every quote is inside one line of that step's screen),
    "引用行なし" or "再生失敗". The same verdicts are printed.
Failure modes: exit 2 for a malformed findings file; exit 1 when any
    finding is not "引用行あり".
Side effects: the driver's throwaway repositories under ./tmp/.
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
FOUND, MISSING, FAILED = "引用行あり", "引用行なし", "再生失敗"


def quotes_of(finding):
    q = finding["quote"]
    return [q] if isinstance(q, str) else list(q)


def check(finding, bin_path, out):
    fid = finding["id"]
    run_dir = out / fid
    p = subprocess.run(
        ["uv", "run", "--quiet", str(ROOT / "scripts/tui-ux/drive.py"),
         "--bin", str(bin_path), "--fixture", finding["fixture"],
         "--size", finding["size"], "--keys", str(finding["keys"]), "--out", str(run_dir)],
        cwd=ROOT, capture_output=True, text=True)
    step = run_dir / f"step-{int(finding['step']):03d}.txt"
    if not step.is_file():
        return {"id": fid, "verdict": FAILED, "exit": p.returncode,
                "detail": (p.stderr or p.stdout).strip()[-500:]}
    lines = [line.rstrip() for line in step.read_text().splitlines()]
    missing = [q for q in quotes_of(finding) if not any(q.rstrip() in line for line in lines)]
    return {"id": fid, "verdict": MISSING if missing else FOUND, "exit": p.returncode,
            "step_file": str(step.relative_to(ROOT)) if step.is_relative_to(ROOT) else step.name,
            "missing": missing}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bin", required=True, type=Path, help="the git-forum binary")
    ap.add_argument("--findings", required=True, type=Path, help="the review's findings.json")
    ap.add_argument("--out", required=True, type=Path, help="output directory (must not exist)")
    args = ap.parse_args()
    if args.out.exists():
        ap.error(f"--out {args.out} already exists")
    try:
        findings = json.loads(args.findings.read_text())["findings"]
        for f in findings:
            missing = {"id", "keys", "size", "fixture", "step", "quote"} - f.keys()
            if missing:
                raise ValueError(f"finding {f.get('id')!r} lacks {sorted(missing)}")
    except (OSError, ValueError, KeyError, TypeError) as e:
        print(f"verify: {e}", file=sys.stderr)
        sys.exit(2)

    args.out.mkdir(parents=True)
    results = [check(f, args.bin.resolve(), args.out.resolve()) for f in findings]
    (args.out / "verify.json").write_text(
        json.dumps({"results": results}, ensure_ascii=False, indent=1) + "\n")
    for r in results:
        extra = f" {r['missing']}" if r.get("missing") else ""
        print(f"{r['id']}: {r['verdict']}{extra}")
    sys.exit(0 if all(r["verdict"] == FOUND for r in results) else 1)


if __name__ == "__main__":
    main()
