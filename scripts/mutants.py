#!/usr/bin/env python3
"""Mutation baseline comparator, driven by the justfile's `mutants-*` recipes.

    mutants.py check OUTDIR MODE STATUS   # MODE: exact (full sweep) | subset (diff run)
    mutants.py update                     # rewrite the baseline from target/mutants-full

`.cargo/mutants-baseline.txt` holds one `survivor  # reason` per line; full-line `#`
comments and blank lines are ignored. Every entry must carry a reason. `:LINE:COL` is
stripped from survivors so ordinary edits do not churn the file, and duplicates are
kept (two identical rewrites in one function are two survivors).
"""
import re
import sys
from pathlib import Path

BASE = Path(".cargo/mutants-baseline.txt")
NEEDED = "REASON NEEDED"


def entries():
    """(survivor, reason) per entry line, plus the header lines before the first entry."""
    head, out = [], []
    for line in BASE.read_text().splitlines(keepends=True):
        s, _, r = line.rstrip("\n").partition("  # ")
        if not s.strip() or s.lstrip().startswith("#"):
            if not out:
                head.append(line)
        else:
            out.append((s.strip(), r.strip()))
    return head, out


def survivors(missed):
    return sorted(re.sub(r"^([^:]+):[0-9]+:[0-9]+: ", r"\1: ", l) for l in missed.read_text().splitlines() if l.strip())


def check(outdir, mode, status):
    # 0: none survived, 2: some did, 3: some timed out and none survived (a
    # non-terminating loop mutant is detected, not missed); anything else is a broken run.
    if status not in (0, 2, 3):
        sys.exit(f"cargo mutants exited {status} — see https://mutants.rs/exit-codes.html")
    missed = Path(outdir) / "mutants.out" / "missed.txt"
    if not missed.exists():
        if mode == "subset":
            print("✓ no mutants were generated for this diff")
            return
        print("A full sweep wrote no missed.txt: a broken run, not a clean one.", file=sys.stderr)
        sys.exit(70)
    if not BASE.exists():
        print(f"{BASE} is missing: restore it rather than regenerating (it carries the reasons).", file=sys.stderr)
        sys.exit(70)
    _, recorded = entries()
    unexplained = [s for s, r in recorded if not r or r == NEEDED]
    if unexplained:
        print("Baseline entries without a reason:\n" + "\n".join("    " + s for s in unexplained), file=sys.stderr)
        sys.exit(1)
    found, base = survivors(missed), sorted(s for s, _ in recorded)
    Path(outdir, "survivors.txt").write_text("".join(s + "\n" for s in found))
    rest = list(base)
    new = [s for s in found if not (s in rest and not rest.remove(s))]
    if mode == "subset":
        if not new:
            print(f"✓ {len(found)} survivor(s) in the mutated range, all recorded in {BASE}")
            return
        print(f"These mutants survived and are NOT recorded in {BASE}:", file=sys.stderr)
        print("\n".join("    " + s for s in new), file=sys.stderr)
        print("Write a test that kills each one, or record it WITH its reason.", file=sys.stderr)
        sys.exit(1)
    if not new and not rest:
        print(f"✓ {len(found)} survivors, every one recorded in {BASE}")
        return
    for s in rest:
        print(f"  - {s}    (recorded, now caught: delete the entry)", file=sys.stderr)
    for s in new:
        print(f"  + {s}    (new survivor: kill it, or record it with its reason)", file=sys.stderr)
    sys.exit(1)


def update():
    full = Path("target/mutants-full/survivors.txt")
    if not full.exists():
        sys.exit("No full sweep to update from: run 'just mutants-gate' first (a diff run sees only part of the baseline).")
    head, recorded = entries() if BASE.exists() else ([], [])
    reasons = {}
    for s, r in recorded:
        reasons.setdefault(s, []).append(r)
    out = []
    for s in full.read_text().splitlines():
        if s.strip():
            r = reasons.get(s, [])
            out.append(f"{s}  # {r.pop(0) if r else NEEDED}\n")
    BASE.write_text("".join(head) + "".join(out))
    todo = sum(NEEDED in l for l in out)
    print(f"{BASE}: {len(out)} entries, reasons kept; {todo} marked '{NEEDED}' (the gate fails until each has one)")


if __name__ == "__main__":
    if sys.argv[1] == "check":
        check(sys.argv[2], sys.argv[3], int(sys.argv[4]))
    else:
        update()
